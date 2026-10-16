// A bare check counter for the lens tests: no dependency, one line per check, a non-zero exit
// on any failure. The same shape as Daimond's `debugshare.test.mjs`, so a reader moving between
// the two sees one dialect.
let passes = 0, failures = 0;

export function check(name, cond, detail) {
	if (cond) { passes += 1; console.log('  ok   ' + name); }
	else { failures += 1; console.log('  FAIL ' + name + (detail ? ' -- ' + detail : '')); }
}

export function section(title) { console.log(title); }

export function counts() { return { passes, failures }; }

export function finish(label) {
	console.log('\n' + label + ': ' + passes + ' ok, ' + failures + ' failed');
	process.exit(failures ? 1 : 0);
}

// Runs one block of checks. A throw inside it is a failed check, not the end of the run, so a
// stub or a half-built core reports how much of the suite it breaks rather than the first fault.
export async function part(fn) {
	try { await fn(); }
	catch (e) { failures += 1; console.log('  FAIL block threw: ' + (e && e.message)); }
}
