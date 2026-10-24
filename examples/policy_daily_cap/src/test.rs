#[cfg(test)]
mod tests {
    use crate::{PolicyDailyCap, PolicyDailyCapClient, DAY_SECS};
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{Address, Env};

    fn setup() -> (Env, PolicyDailyCapClient<'static>, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, PolicyDailyCap);
        let client = PolicyDailyCapClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        (env, client, admin, user)
    }

    #[test]
    fn test_initialize_sets_cap() {
        let (_env, client, admin, _user) = setup();
        client.initialize(&admin, &1_000i128);
        assert_eq!(client.get_daily_cap(), 1_000i128);
    }

    #[test]
    fn test_initialize_rejects_non_positive_cap() {
        let (_env, client, admin, _user) = setup();
        let res = client.try_initialize(&admin, &0i128);
        assert!(res.is_err());
        let res = client.try_initialize(&admin, &-5i128);
        assert!(res.is_err());
    }

    #[test]
    fn test_initialize_twice_fails() {
        let (_env, client, admin, _user) = setup();
        client.initialize(&admin, &1_000i128);
        let res = client.try_initialize(&admin, &2_000i128);
        assert!(res.is_err());
    }

    #[test]
    fn test_set_daily_cap_validation() {
        let (_env, client, admin, _user) = setup();
        client.initialize(&admin, &1_000i128);
        client.set_daily_cap(&admin, &2_000i128);
        assert_eq!(client.get_daily_cap(), 2_000i128);

        let res = client.try_set_daily_cap(&admin, &0i128);
        assert!(res.is_err());
        let res = client.try_set_daily_cap(&admin, &-1i128);
        assert!(res.is_err());
    }

    #[test]
    fn test_accumulate_up_to_cap() {
        let (_env, client, admin, user) = setup();
        client.initialize(&admin, &1_000i128);

        assert!(client.try_record(&user, &400i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 400i128);

        assert!(client.try_record(&user, &600i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 1_000i128);
    }

    #[test]
    fn test_reject_past_cap_without_mutation() {
        let (_env, client, admin, user) = setup();
        client.initialize(&admin, &1_000i128);

        assert!(client.try_record(&user, &900i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 900i128);

        let res = client.try_record(&user, &200i128);
        assert!(res.is_err());
        // State must be unchanged after a rejected record.
        assert_eq!(client.get_user_volume(&user), 900i128);

        // Exactly reaching the cap is still allowed.
        assert!(client.try_record(&user, &100i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 1_000i128);
    }

    #[test]
    fn test_reject_non_positive_amount() {
        let (_env, client, admin, user) = setup();
        client.initialize(&admin, &1_000i128);

        let res = client.try_record(&user, &0i128);
        assert!(res.is_err());
        let res = client.try_record(&user, &-100i128);
        assert!(res.is_err());
        assert_eq!(client.get_user_volume(&user), 0i128);
    }

    #[test]
    fn test_window_reset_at_day_boundary() {
        let (env, client, admin, user) = setup();
        client.initialize(&admin, &1_000i128);

        env.ledger().set_timestamp(1_000);
        assert!(client.try_record(&user, &1_000i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 1_000i128);

        // Still within the same 24h window: further volume is rejected.
        env.ledger().set_timestamp(1_000 + DAY_SECS - 1);
        let res = client.try_record(&user, &1i128);
        assert!(res.is_err());
        assert_eq!(client.get_user_volume(&user), 1_000i128);

        // At exactly the boundary the window rolls over and volume resets.
        env.ledger().set_timestamp(1_000 + DAY_SECS);
        assert_eq!(client.get_user_volume(&user), 0i128);
        assert!(client.try_record(&user, &1_000i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 1_000i128);
    }

    #[test]
    fn test_get_user_volume_unknown_user() {
        let (_env, client, admin, user) = setup();
        client.initialize(&admin, &1_000i128);
        assert_eq!(client.get_user_volume(&user), 0i128);
    }

    #[test]
    fn test_get_user_volume_expired_window() {
        let (env, client, admin, user) = setup();
        client.initialize(&admin, &1_000i128);

        env.ledger().set_timestamp(5_000);
        assert!(client.try_record(&user, &300i128).is_ok());
        assert_eq!(client.get_user_volume(&user), 300i128);

        env.ledger().set_timestamp(5_000 + DAY_SECS);
        assert_eq!(client.get_user_volume(&user), 0i128);
    }

    #[test]
    fn test_per_user_independence() {
        let (env, client, admin, user_a) = setup();
        let user_b = Address::generate(&env);
        client.initialize(&admin, &1_000i128);

        assert!(client.try_record(&user_a, &1_000i128).is_ok());
        assert_eq!(client.get_user_volume(&user_a), 1_000i128);
        assert_eq!(client.get_user_volume(&user_b), 0i128);

        // user_a is capped out but user_b is unaffected.
        assert!(client.try_record(&user_a, &1i128).is_err());
        assert!(client.try_record(&user_b, &1_000i128).is_ok());
        assert_eq!(client.get_user_volume(&user_b), 1_000i128);
    }
}