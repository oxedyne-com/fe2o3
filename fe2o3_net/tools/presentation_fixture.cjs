// Captures a named presentation signed by a real WebCrypto Ed25519 key in
// headless Chromium, for fe2o3_net's tests/presentation.rs. The browser makes
// both keys (an issuer's and a member's) non-extractable, signs the head and the
// presentation over its own canonical JSON (keys sorted, JSON.stringify, no
// whitespace), and hands back only public material. The Rust side must accept
// the lot, which it can only do if it rebuilds the browser's bytes exactly and
// verifies the browser's signatures.
//
// Run from fe2o3_net, with NODE_PATH naming a node_modules that holds playwright,
// and CHROMIUM_PATH naming a Chromium binary when playwright's own is absent:
//
//     NODE_PATH=<dir>/node_modules node tools/presentation_fixture.cjs \
//         > tests/data/presentation_webcrypto.json

'use strict';

const { chromium } = require('playwright');

(async () => {
    const browser = await chromium.launch({
        headless:       true,
        executablePath: process.env.CHROMIUM_PATH || undefined,
    });
    const page = await browser.newPage();
    // WebCrypto lives only in a secure context, so serve a blank page on https.
    await page.route('https://fixture.test/**', route => route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<!doctype html><title>fixture</title>',
    }));
    await page.goto('https://fixture.test/');

    const out = await page.evaluate(async () => {
        const enc = new TextEncoder();
        const sorted = v => Array.isArray(v) ? v.map(sorted)
            : (v && typeof v === 'object')
                ? Object.fromEntries(Object.keys(v).sort().map(k => [k, sorted(v[k])]))
                : v;
        const jcs = v => JSON.stringify(sorted(v));
        const b64u = bytes => btoa(String.fromCharCode(...bytes))
            .replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        const hex = bytes => [...bytes].map(b => b.toString(16).padStart(2, '0')).join('');
        const sha256 = async bytes => new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
        const keyPair = () => crypto.subtle.generateKey({ name: 'Ed25519' }, false, ['sign', 'verify']);
        const raw = async pair => new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
        const sign = async (pair, text) => new Uint8Array(
            await crypto.subtle.sign({ name: 'Ed25519' }, pair.privateKey, enc.encode(text)));

        const issuer = await keyPair();
        const member = await keyPair();
        const issuerPub = await raw(issuer);
        const memberPub = await raw(member);
        const now = Math.floor(Date.now() / 1000);
        const rp_id = 'https://app.example';
        const nonce = crypto.getRandomValues(new Uint8Array(32));

        // A head with an empty ring list, which is what a named presentation
        // needs and what a network with no ring keys yet serves.
        const headBody = {
            v: 'head/1', epoch: 1, prev: null, ts: now - 30,
            salt: b64u(crypto.getRandomValues(new Uint8Array(16))),
            members: 1, ring_n: 0, ring_digest: b64u(await sha256(new Uint8Array(0))),
            signer: b64u(issuerPub),
        };
        const headText = jcs(headBody);
        const headId = await sha256(enc.encode(headText));
        const head = { ...headBody, head: b64u(headId), sig: b64u(await sign(issuer, headText)) };

        const body = {
            v: 'present/1', mode: 'named', rp_id, nonce: b64u(nonce),
            sub: hex((await sha256(memberPub)).slice(0, 5)), pub: b64u(memberPub),
            predicates: ['adult'], head: b64u(headId), ts: now,
        };
        const text = jcs(body);
        const presentation = JSON.stringify({ ...body, sig: b64u(await sign(member, text)) });

        return {
            browser: navigator.userAgent,
            now,
            rp_id,
            request: {
                v: 'present-req/1', rp_id, nonce: b64u(nonce),
                modes: ['pairwise', 'named'], predicates: ['adult'], exp: now + 300,
            },
            head,
            presentation,
            status: { pub: b64u(memberPub), live: true },
        };
    });

    process.stdout.write(JSON.stringify(out, null, 1) + '\n');
    await browser.close();
})().catch(e => {
    process.stderr.write(String(e && e.stack || e) + '\n');
    process.exit(1);
});
