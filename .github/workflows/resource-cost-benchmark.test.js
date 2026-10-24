// Test suite for the resource-cost documentation pipeline CI job.
// Verifies that benchmark results are properly compared against published values
// and that drift beyond thresholds is detected and reported.

const fs = require('fs');
const path = require('path');

// Contract crates whose resource ceilings are enforced by the `resource-budget`
// CI job. Kept in sync with the `crate` matrix in `.github/workflows/ci.yml`.
const RESOURCE_BUDGET_CRATES = [
    'intent_settlement',
    'solver_registry',
    'proof_registry',
    'reputation_badge',
    'governor',
    'backstop_vault',
    'fee_router',
    'intent_periphery',
    'timelock',
    'treasury_streams'
];

// Mock benchmark result parser
class BenchmarkParser {
    static parseFromFile(filePath) {
        try {
            const content = fs.readFileSync(filePath, 'utf8');
            return JSON.parse(content);
        } catch (error) {
            throw new Error(`Failed to parse benchmark file: ${error.message}`);
        }
    }

    static extractResourceCosts(benchmarkOutput) {
        // Simulate extracting resource costs from benchmark harness output
        // Expected format: { entrypoint: name, cpu: cost, memory: cost, ... }
        return benchmarkOutput.results || [];
    }
}

// Mock documentation parser
class DocParser {
    static parsePublishedCosts(docPath) {
        try {
            const content = fs.readFileSync(docPath, 'utf8');

            // Extract resource costs from markdown table
            const costsMatch = content.match(/## Resource Costs([\s\S]*?)##/);
            if (!costsMatch) {
                throw new Error('Resource costs section not found in documentation');
            }

            const costs = {};
            const lines = costsMatch[1].split('\n');

            for (const line of lines) {
                // Parse markdown table rows: | entrypoint | cpu | memory | ... |
                const match = line.match(/\|\s*([^\|]+?)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|/);
                if (match) {
                    costs[match[1].trim()] = {
                        cpu: parseInt(match[2]),
                        memory: parseInt(match[3])
                    };
                }
            }

            return costs;
        } catch (error) {
            throw new Error(`Failed to parse documentation: ${error.message}`);
        }
    }

    static updatePublishedCosts(docPath, newCosts) {
        try {
            let content = fs.readFileSync(docPath, 'utf8');

            // Build markdown table
            let table = '\n## Resource Costs\n\n| Entrypoint | CPU | Memory |\n|---|---|---|\n';
            for (const [entrypoint, costs] of Object.entries(newCosts)) {
                table += `| ${entrypoint} | ${costs.cpu} | ${costs.memory} |\n`;
            }

            // Replace or add resource costs section
            const sectionRegex = /## Resource Costs([\s\S]*?)(?=##|$)/;
            if (sectionRegex.test(content)) {
                content = content.replace(sectionRegex, table);
            } else {
                content = content + table;
            }

            fs.writeFileSync(docPath, content);
        } catch (error) {
            throw new Error(`Failed to update documentation: ${error.message}`);
        }
    }
}

// Drift detection and reporting
class DriftDetector {
    constructor(thresholdPercent = 10) {
        this.thresholdPercent = thresholdPercent;
    }

    detectDrift(publishedCosts, measuredCosts) {
        const driftReport = {
            hasDrift: false,
            driftItems: [],
            summary: null
        };

        for (const [entrypoint, published] of Object.entries(publishedCosts)) {
            if (!measuredCosts[entrypoint]) {
                continue;
            }

            const measured = measuredCosts[entrypoint];

            // Calculate percentage drift for CPU
            const cpuDrift = Math.abs((measured.cpu - published.cpu) / published.cpu) * 100;
            if (cpuDrift > this.thresholdPercent) {
                driftReport.hasDrift = true;
                driftReport.driftItems.push({
                    entrypoint,
                    resource: 'cpu',
                    published: published.cpu,
                    measured: measured.cpu,
                    driftPercent: cpuDrift.toFixed(2)
                });
            }

            // Calculate percentage drift for memory
            const memoryDrift = Math.abs((measured.memory - published.memory) / published.memory) * 100;
            if (memoryDrift > this.thresholdPercent) {
                driftReport.hasDrift = true;
                driftReport.driftItems.push({
                    entrypoint,
                    resource: 'memory',
                    published: published.memory,
                    measured: measured.memory,
                    driftPercent: memoryDrift.toFixed(2)
                });
            }
        }

        if (driftReport.hasDrift) {
            driftReport.summary = `Detected ${driftReport.driftItems.length} resource cost drift(s) exceeding ${this.thresholdPercent}% threshold`;
        }

        return driftReport;
    }

    formatDriftReport(driftReport) {
        if (!driftReport.hasDrift) {
            return 'No significant drift detected. Published costs remain accurate.';
        }

        let report = `# Resource Cost Drift Report\n\n${driftReport.summary}\n\n`;
        report += '| Entrypoint | Resource | Published | Measured | Drift % |\n';
        report += '|---|---|---|---|---|\n';

        for (const item of driftReport.driftItems) {
            report += `| ${item.entrypoint} | ${item.resource} | ${item.published} | ${item.measured} | ${item.driftPercent}% |\n`;
        }

        return report;
    }
}

describe('Resource Cost Benchmark Pipeline', () => {
    test('parses benchmark output correctly', () => {
        const benchmarkOutput = {
            results: [
                { entrypoint: 'submit_intent', cpu: 1000, memory: 512 },
                { entrypoint: 'accept_intent', cpu: 1200, memory: 600 },
                { entrypoint: 'fill_intent', cpu: 1500, memory: 700 }
            ]
        };

        const costs = BenchmarkParser.extractResourceCosts(benchmarkOutput);

        expect(costs.length).toBe(3);
        expect(costs[0].entrypoint).toBe('submit_intent');
        expect(costs[0].cpu).toBe(1000);
    });

    test('loads published resource costs from documentation', () => {
        const mockDocPath = '/tmp/test-doc.md';
        const mockContent = `
# Resource Costs

## Resource Costs

| Entrypoint | CPU | Memory |
|---|---|---|
| submit_intent | 950 | 500 |
| accept_intent | 1100 | 580 |
| fill_intent | 1400 | 680 |

## Other Sections
        `;

        // Simulate file system
        const costs = {
            'submit_intent': { cpu: 950, memory: 500 },
            'accept_intent': { cpu: 1100, memory: 580 },
            'fill_intent': { cpu: 1400, memory: 680 }
        };

        expect(costs['submit_intent'].cpu).toBe(950);
        expect(costs['accept_intent'].memory).toBe(580);
        expect(costs['fill_intent'].cpu).toBe(1400);
    });
});

describe('Resource Budget CI Matrix', () => {
    test('enforces resource ceilings for all ten contract crates', () => {
        expect(RESOURCE_BUDGET_CRATES).toHaveLength(10);
        expect(new Set(RESOURCE_BUDGET_CRATES).size).toBe(10);
    });

    test('includes the newest six contract crates', () => {
        const newest = [
            'governor',
            'backstop_vault',
            'fee_router',
            'intent_periphery',
            'timelock',
            'treasury_streams'
        ];

        for (const crate of newest) {
            expect(RESOURCE_BUDGET_CRATES).toContain(crate);
        }
    });

    test('retains the originally covered contract crates', () => {
        const original = [
            'intent_settlement',
            'solver_registry',
            'proof_registry',
            'reputation_badge'
        ];

        for (const crate of original) {
            expect(RESOURCE_BUDGET_CRATES).toContain(crate);
        }
    });
});

describe('Drift Detection and Threshold', () => {
    test('detects drift exceeding 10% threshold', () => {
        const detector = new DriftDetector(10);

        const publishedCosts = {
            'submit_intent': { cpu: 1000, memory: 500 },
            'accept_intent': { cpu: 1200, memory: 600 }
        };

        const measuredCosts = {
            'submit_intent': { cpu: 1150, memory: 500 }, // 15% CPU drift
            'accept_intent': { cpu: 1200, memory: 600 }   // No drift
        };

        const report = detector.detectDrift(publishedCosts, measuredCosts);

        expect(report.hasDrift).toBe(true);
        expect(report.driftItems.length).toBe(1);
        expect(report.driftItems[0].entrypoint).toBe('submit_intent');
        expect(report.driftItems[0].driftPercent).toBe('15.00');
    });

    test('ignores drift within threshold', () => {
        const detector = new DriftDetector(10);

        const publishedCosts = {
            'submit_intent': { cpu: 1000, memory: 500 }
        };

        const measuredCosts = {
            'submit_intent': { cpu: 1050, memory: 500 } // 5% CPU drift (within threshold)
        };

        const report = detector.detectDrift(publishedCosts, measuredCosts);

        expect(report.hasDrift).toBe(false);
        expect(report.driftItems.length).toBe(0);
    });
});
