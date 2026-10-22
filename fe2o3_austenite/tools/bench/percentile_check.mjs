#!/usr/bin/env node
// Check: the bench's percentile is nearest-rank, and its typing threshold is max(1.25 t, t + 16 ms). Over the values 1..50, p50 is the 25th smallest
// and p95 the 48th (a `floor(p * n / 100)` index gives 26 and 48, so the p50 case is the one a
// floor plant turns red). The other cases fix the rank at the ends and at a count where
// `p / 100 * n` is not exact in floating point.
import { nearestRank, median, typingThreshold, floorBinds } from './lib/stats.mjs';

const seq = (n) => Array.from({ length: n }, (_, i) => i + 1);
let ran = 0;
let failed = 0;
function expect(what, got, want) {
	ran += 1;
	if (got !== want) {
		failed += 1;
		process.stdout.write(`FAIL ${what}: got ${got}, want ${want}\n`);
	} else {
		process.stdout.write(`ok   ${what} = ${got}\n`);
	}
}

const s50 = seq(50);
expect('p50 of 1..50', nearestRank(s50, 50), 25);
expect('p95 of 1..50', nearestRank(s50, 95), 48);
expect('p100 of 1..50', nearestRank(s50, 100), 50);
expect('p0 of 1..50 (clamps to the minimum)', nearestRank(s50, 0), 1);
expect('p50 of 1..10', nearestRank(seq(10), 50), 5);
expect('p95 of 1..10', nearestRank(seq(10), 95), 10);
expect('p95 of 1..100', nearestRank(seq(100), 95), 95);
expect('p7 of 1..100 (0.07 * 100 is not exact in floating point)', nearestRank(seq(100), 7), 7);
expect('p50 of one value', nearestRank([42], 50), 42);
expect('p95 of one value', nearestRank([42], 95), 42);
expect('p50 of nothing', nearestRank([], 50), null);
expect('median of an odd count', median([3, 1, 2]), 2);
expect('median of an even count', median([4, 1, 3, 2]), 2.5);

// The typing threshold: the 16 ms floor governs under 64 ms, the 1.25x ratio above it.
const near = (a, b) => Math.abs(a - b) < 1e-12;
expect('threshold at 10 ms is 26 ms (floor governs)', near(typingThreshold(0.010), 0.026), true);
expect('threshold at 100 ms is 125 ms (ratio governs)', near(typingThreshold(0.100), 0.125), true);
expect('the floor binds at 40 ms', floorBinds(0.040), true);
expect('the floor does not bind at 100 ms', floorBinds(0.100), false);

process.stdout.write(`percentile_check: ${ran} checks, ${failed} failed\n`);
process.exit(failed ? 1 : 0);
