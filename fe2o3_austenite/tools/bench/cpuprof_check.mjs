#!/usr/bin/env node
// Checks cpuprof_rank.mjs on a profile built by hand, whose every figure is known: the demangling of
// legacy Rust symbols, a sample lasting until the next sample, the nearest Austenite frame taking the time
// of the allocator frames beneath it, and an inclusive time counted once whatever the recursion.
import { demangle, rank } from './cpuprof_rank.mjs';

let failed = 0;
let checks = 0;
function check(name, got, want) {
	checks++;
	const g = JSON.stringify(got);
	const w = JSON.stringify(want);
	if (g !== w) {
		failed++;
		console.log(`FAIL ${name}: got ${g}, want ${w}`);
	}
}

const H = 'h0123456789abcdef';
const sym = (...parts) => `_ZN${parts.map((p) => p.length + p).join('')}${17}${H}E`;
check('plain path', demangle(sym('oxedyne_fe2o3_austenite', 'flow', 'par', 'layout_par')),
	'oxedyne_fe2o3_austenite::flow::par::layout_par');
check('trait impl', demangle(`_ZN67_$LT$alloc..vec..Vec$LT$T$C$A$GT$$u20$as$u20$core..clone..Clone$GT$5clone17${H}E`),
	'<alloc::vec::Vec<T,A> as core::clone::Clone>::clone');
check('not a symbol', demangle('compileProject'), 'compileProject');
check('already demangled', demangle(`oxedyne_fe2o3_austenite::flow::par::layout_with::${H}`),
	'oxedyne_fe2o3_austenite::flow::par::layout_with');

const frame = (id, functionName, children = [], url = 'wasm://wasm/x') => ({ id, callFrame: { functionName, url }, children });
const A = (m, f) => sym('oxedyne_fe2o3_austenite', m, f);
// root(1) > austenite flow::par (2) > alloc::vec::push (3) and > austenite flow::par recursion (4) > memcpy-like core::ptr (5);
// root > austenite eval::eval (6).
const profile = {
	startTime: 0, endTime: 100000,
	nodes: [
		frame(1, '(root)', [2, 6], ''),
		frame(2, A('flow', 'par'), [3, 4]),
		frame(3, sym('alloc', 'vec', 'push')),
		frame(4, A('flow', 'par'), [5]),
		frame(5, sym('core', 'ptr', 'copy')),
		frame(6, A('eval', 'eval')),
	],
	// Samples at 0, 10, 30, 60 and 100 ms: nodes 3, 5, 5, 6, 3. Each lasts until the next, the last not at all,
	// so they last 10, 20, 30, 40 and 0 ms; read as the delta before each they would last 0, 10, 20, 30, 40.
	samples:	[3, 5, 5, 6, 3],
	timeDeltas:	[0, 10000, 20000, 30000, 40000],
};
const r = rank(profile, { top: 5, incl: ['flow::par', 'eval::eval'], callers: ['core::ptr::copy'] });
check('busy', r.total_ms, 100);
const by = (rows) => Object.fromEntries(rows.map((x) => [x.name, x.ms]).sort((a, b) => a[0].localeCompare(b[0])));
check('self by crate', by(r.by_crate), { alloc: 10, core: 50, oxedyne_fe2o3_austenite: 40 });
check('asked by', by(r.asked_by), { 'austenite::eval::eval': 40, 'austenite::flow::par': 60 });
check('inclusive counts once', by(r.inclusive), { 'eval::eval': 40, 'flow::par': 60 });
// Node 5 (core::ptr::copy, 50 ms) is called by node 4 (flow::par), whose own name does not match.
check('callers', by(r.callers[0].rows), { 'oxedyne_fe2o3_austenite::flow::par': 50 });
console.log(`cpuprof_check: ${checks} checks, ${failed} failed`);
process.exit(failed ? 1 : 0);
