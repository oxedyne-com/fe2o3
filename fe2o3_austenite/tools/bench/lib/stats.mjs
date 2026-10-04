// Order statistics shared by the node-side bench tools.

/// The nearest-rank percentile of an ascending array: the value at 1-based rank ceil(p * n / 100),
/// so p50 of 1..50 is the 25th smallest and p95 the 48th. `null` for an empty array. `p * n` stays
/// an integer for an integer `p`, so the division is exact where the rank is whole.
export function nearestRank(sorted, p) {
	const n = sorted.length;
	if (!n) return null;
	const rank = Math.min(n, Math.max(1, Math.ceil((p * n) / 100)));
	return sorted[rank - 1];
}

/// The median of an unsorted array of numbers (the mean of the middle pair for an even count).
export function median(values) {
	const s = [...values].sort((a, b) => a - b);
	const n = s.length;
	if (!n) return null;
	return n % 2 ? s[(n - 1) / 2] : (s[n / 2 - 1] + s[n / 2]) / 2;
}

/// The typing test's threshold for one percentile of typst.ts's time `t` (seconds):
/// max(1.25 * t, t + 16 ms), per D-20261001-08. One 60 Hz frame is the floor at which a
/// difference stops being visible, so the additive term governs while t is under 64 ms.
export function typingThreshold(t) {
	return Math.max(1.25 * t, t + 0.016);
}

/// Does the 16 ms floor, rather than the 1.25x ratio, set the threshold at `t`?
export function floorBinds(t) {
	return t + 0.016 > 1.25 * t;
}
