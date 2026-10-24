"""
pytest suite for tools/signal_aggregator.py — issue #511.

Focuses on ``SignalAggregator._verify_signature``, which previously accepted
any non-empty string as a valid signature (a no-op placeholder).  These tests
assert that:

  1. A genuine Ed25519 signature over the canonical payload is accepted.
  2. A tampered/forged signature is rejected — the critical regression test
     that the old placeholder would have *incorrectly* passed.
  3. An empty signature is rejected.
  4. An empty address is rejected.
  5. A signature that is valid bytes but signed by a *different* keypair is
     rejected (wrong-key forgery).
  6. A signature whose base64 encoding is malformed is rejected.

All tests are fully offline; no Stellar RPC calls are made.

Dependencies (same as the tool itself):
    pip install stellar-sdk PyNaCl
"""

import base64
import json
import sys
from pathlib import Path

import pytest

# ---------------------------------------------------------------------------
# Path setup — allow running pytest from either the repo root or tools/tests/
# ---------------------------------------------------------------------------
_TOOLS_DIR = Path(__file__).resolve().parent.parent
if str(_TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(_TOOLS_DIR))

from signal_aggregator import SignalAggregator  # noqa: E402  (import after sys.path tweak)


# ---------------------------------------------------------------------------
# Helpers — generate real Ed25519 key material via stellar_sdk + PyNaCl
# ---------------------------------------------------------------------------

def _make_keypair():
    """Return a (Keypair, VerifyKey) pair using stellar_sdk."""
    pytest.importorskip("stellar_sdk", reason="stellar-sdk not installed; skipping crypto tests")
    from stellar_sdk import Keypair
    return Keypair.random()


def _sign(keypair, message: str) -> str:
    """
    Sign *message* (a UTF-8 string) with *keypair* and return a base64-encoded
    Ed25519 signature.  Mirrors exactly what a real governance client would do.
    """
    from nacl.signing import SigningKey
    from stellar_sdk import StrKey

    # Recover the raw 32-byte seed from the keypair's secret key.
    raw_seed: bytes = StrKey.decode_ed25519_secret_seed(keypair.secret)
    signing_key = SigningKey(raw_seed)
    signed = signing_key.sign(message.encode("utf-8"))
    # signed.signature is the 64-byte raw Ed25519 signature.
    return base64.b64encode(signed.signature).decode("ascii")


def _canonical(payload: dict) -> str:
    """Reproduce the canonicalization used by SignalAggregator."""
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

SAMPLE_PAYLOAD = {
    "message": "Audit is critical.",
    "proposal_id": "proposal-2026-10-15-treasury-audit",
    "proposal_title": "Fund Security Audit Q4 2026",
    "signal": "for",
    "timestamp": 1698067200,
    "voter_address": "PLACEHOLDER",  # overwritten per test
}


class TestVerifySignature:
    """Unit tests for SignalAggregator._verify_signature (issue #511)."""

    def test_valid_signature_accepted(self):
        """A genuine signature over the canonical payload must be accepted."""
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key}
        canonical = _canonical(payload)
        sig = _sign(kp, canonical)

        assert SignalAggregator._verify_signature(canonical, sig, kp.public_key) is True

    def test_tampered_signature_rejected(self):
        """
        A forged / tampered signature must be rejected.

        This is the critical regression test: the old placeholder returned True
        for *any* non-empty signature, which this test would have caught.
        """
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key}
        canonical = _canonical(payload)
        real_sig = _sign(kp, canonical)

        # Flip the last byte of the raw signature to produce a forgery.
        raw = bytearray(base64.b64decode(real_sig))
        raw[-1] ^= 0xFF
        tampered_sig = base64.b64encode(bytes(raw)).decode("ascii")

        assert SignalAggregator._verify_signature(canonical, tampered_sig, kp.public_key) is False

    def test_wrong_key_forgery_rejected(self):
        """A signature valid for keypair A must not verify against keypair B's address."""
        pytest.importorskip("stellar_sdk")
        kp_signer = _make_keypair()
        kp_claimed = _make_keypair()

        payload = {**SAMPLE_PAYLOAD, "voter_address": kp_claimed.public_key}
        canonical = _canonical(payload)
        # Sign with kp_signer but claim kp_claimed as the author.
        sig = _sign(kp_signer, canonical)

        assert SignalAggregator._verify_signature(canonical, sig, kp_claimed.public_key) is False

    def test_empty_signature_rejected(self):
        """An empty signature string must be rejected without raising."""
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key}
        canonical = _canonical(payload)

        assert SignalAggregator._verify_signature(canonical, "", kp.public_key) is False

    def test_empty_address_rejected(self):
        """An empty address must be rejected without raising."""
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key}
        canonical = _canonical(payload)
        sig = _sign(kp, canonical)

        assert SignalAggregator._verify_signature(canonical, sig, "") is False

    def test_malformed_base64_rejected(self):
        """A signature that is not valid base64 must be rejected without raising."""
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key}
        canonical = _canonical(payload)

        assert SignalAggregator._verify_signature(canonical, "not!valid!base64!!!", kp.public_key) is False

    def test_signature_over_wrong_message_rejected(self):
        """A valid signature over payload A must not verify against a different payload B."""
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload_a = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key, "signal": "for"}
        payload_b = {**SAMPLE_PAYLOAD, "voter_address": kp.public_key, "signal": "against"}

        sig_a = _sign(kp, _canonical(payload_a))

        # Verify sig_a against payload_b's canonical form — must fail.
        assert SignalAggregator._verify_signature(_canonical(payload_b), sig_a, kp.public_key) is False


class TestAddSignalIntegration:
    """
    Integration tests for SignalAggregator.add_signal using real crypto.

    Exercises the full path from add_signal → _verify_signature so that a
    regression to the placeholder would surface at the integration layer too.
    """

    def test_add_signal_accepts_valid_signature(self):
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {
            "proposal_id": "prop-001",
            "voter_address": kp.public_key,
            "signal": "for",
            "timestamp": 1698067200,
            "message": "Ship it.",
        }
        canonical = SignalAggregator._canonicalize(payload)
        sig = _sign(kp, canonical)

        agg = SignalAggregator(contract_id="CTEST", rpc_url="https://example.com")
        ok, msg = agg.add_signal(payload, sig)
        assert ok is True, f"Expected acceptance, got: {msg}"

    def test_add_signal_rejects_forged_signature(self):
        """
        The old placeholder accepted any non-empty signature; this integration
        test confirms the gate now rejects a forged one end-to-end.
        """
        pytest.importorskip("stellar_sdk")
        kp = _make_keypair()
        payload = {
            "proposal_id": "prop-002",
            "voter_address": kp.public_key,
            "signal": "against",
            "timestamp": 1698067200,
            "message": "Too risky.",
        }
        forged_sig = base64.b64encode(b"\x00" * 64).decode("ascii")  # 64 zero bytes

        agg = SignalAggregator(contract_id="CTEST", rpc_url="https://example.com")
        ok, msg = agg.add_signal(payload, forged_sig)
        assert ok is False, "Forged signature must be rejected"
        assert "Signature verification failed" in msg
