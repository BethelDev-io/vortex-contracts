const { SolverRegistryIndexer } = require('./reference-indexer');

describe('SolverRegistryIndexer', () => {
    it('replays the solver lifecycle', () => {
        const idx = new SolverRegistryIndexer();
        idx.replay([
            { topics: ['solver_registered', 'S1'], value: 1000 },
            { topics: ['staked', 'S1'], value: [500, 1500] },
            { topics: ['fill_recorded', 'S1'], value: [300, 9000] },
            { topics: ['failure_recorded', 'S1'], value: 8000 },
            { topics: ['obligation_locked', 'S1'], value: ['id', 2] },
            { topics: ['solver_slashed', 'S1'], value: [150, 1] },
            { topics: ['unstaked', 'S1'], value: [100, 1250] },
            { topics: ['admin_transferred'], value: ['a', 'b'] },
        ]);
        expect(idx.errors).toEqual([]);
        expect(idx.solvers.get('S1')).toMatchObject({
            bondAmount: 1250n, tier: 1, fillsCompleted: 1,
            fillsFailed: 1, totalVolume: 300n, lockedObligations: 2,
        });
        idx.replay([{ topics: ['solver_deregistered', 'S1'], value: 1250 }]);
        expect(idx.solvers.has('S1')).toBe(false);
    });

    it('records errors for events on unknown solvers', () => {
        const idx = new SolverRegistryIndexer();
        idx.replay([{ topics: ['staked', 'nope'], value: [1, 1] }]);
        expect(idx.errors).toHaveLength(1);
    });
});
