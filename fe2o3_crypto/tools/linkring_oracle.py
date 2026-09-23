#!/usr/bin/env python3
"""Independent reference implementation of linkring/1, used only as a test
oracle for fe2o3_crypto::linkring. Never a runtime dependency.

Everything here is written from the specifications, not from the Rust code:
ristretto255 from RFC 9496, hash_to_ristretto255 from RFC 9380, and the
one-out-of-many proof from Bootle et al. (ESORICS 2015) with the tag relation
described in the Rust module header. The prover computes each G_k by plain
polynomial expansion over every padded entry, with none of the Rust prover's
subset expansion or padding fold, so agreement is evidence rather than
self-consistency.

Usage:
    linkring_oracle.py selftest
    linkring_oracle.py sign   < cases     (case blocks, see parse_blocks)
    linkring_oracle.py verify < cases
    linkring_oracle.py vectors            (writes golden vectors to stdout)
"""
import hashlib
import sys

# ── Field and group (RFC 9496) ──────────────────────────────────────────────

P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493


def inv(a):
    return pow(a % P, P - 2, P)


D = (-121665 * inv(121666)) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def is_neg(a):
    return (a % P) & 1


def ct_abs(a):
    a %= P
    return (P - a) % P if is_neg(a) else a


def sqrt_ratio_m1(u, v):
    u %= P
    v %= P
    v3 = v * v % P * v % P
    v7 = v3 * v3 % P * v % P
    r = (u * v3) % P * pow(u * v7 % P, (P - 5) // 8, P) % P
    check = v * r % P * r % P
    correct = check == u
    flipped = check == (-u) % P
    flipped_i = check == (-u * SQRT_M1) % P
    if flipped or flipped_i:
        r = r * SQRT_M1 % P
    r = ct_abs(r)
    return (correct or flipped), r


def _sqrt(a):
    ok, r = sqrt_ratio_m1(a, 1)
    assert ok
    return r


# RFC 9496 section 4.1 constants, checked against their defining relations.
SQRT_AD_MINUS_ONE = 25063068953384623474111414158702152701244531502492656460079210482610430750235
INVSQRT_A_MINUS_D = 54469307008909316920995813868745141605393597292927456921205312896311721017578
ONE_MINUS_D_SQ = (1 - D * D) % P
D_MINUS_ONE_SQ = (D - 1) * (D - 1) % P
assert SQRT_AD_MINUS_ONE * SQRT_AD_MINUS_ONE % P == (-D - 1) % P
assert INVSQRT_A_MINUS_D * INVSQRT_A_MINUS_D % P * ((-1 - D) % P) % P == 1

IDENT = (0, 1, 1, 0)


def add(p1, p2):
    x1, y1, z1, t1 = p1
    x2, y2, z2, t2 = p2
    a = (y1 - x1) * (y2 - x2) % P
    b = (y1 + x1) * (y2 + x2) % P
    c = t1 * 2 * D % P * t2 % P
    d = z1 * 2 * z2 % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def neg(p1):
    x, y, z, t = p1
    return ((-x) % P, y, z, (-t) % P)


def mul(k, pt):
    k %= L
    acc = IDENT
    for bit in reversed(range(k.bit_length())):
        acc = add(acc, acc)
        if (k >> bit) & 1:
            acc = add(acc, pt)
    return acc


def eq(p1, p2):
    x1, y1, _, _ = p1
    x2, y2, _, _ = p2
    return (x1 * y2 - y1 * x2) % P == 0 or (y1 * y2 - x1 * x2) % P == 0


def is_ident(pt):
    return eq(pt, IDENT)


def encode(pt):
    x0, y0, z0, t0 = pt
    u1 = (z0 + y0) * (z0 - y0) % P
    u2 = x0 * y0 % P
    _, invsqrt = sqrt_ratio_m1(1, u1 * u2 % P * u2 % P)
    den1 = invsqrt * u1 % P
    den2 = invsqrt * u2 % P
    z_inv = den1 * den2 % P * t0 % P
    ix0 = x0 * SQRT_M1 % P
    iy0 = y0 * SQRT_M1 % P
    ench = den1 * INVSQRT_A_MINUS_D % P
    rotate = is_neg(t0 * z_inv)
    if rotate:
        x, y, den_inv = iy0, ix0, ench
    else:
        x, y, den_inv = x0, y0, den2
    if is_neg(x * z_inv):
        y = (-y) % P
    s = ct_abs(den_inv * (z0 - y))
    return s.to_bytes(32, "little")


def decode(b):
    if len(b) != 32:
        return None
    s = int.from_bytes(b, "little")
    if s >= P or is_neg(s):
        return None
    ss = s * s % P
    u1 = (1 - ss) % P
    u2 = (1 + ss) % P
    u2s = u2 * u2 % P
    v = (-(D * u1 % P * u1) - u2s) % P
    ok, invsqrt = sqrt_ratio_m1(1, v * u2s % P)
    den_x = invsqrt * u2 % P
    den_y = invsqrt * den_x % P * v % P
    x = ct_abs(2 * s * den_x)
    y = u1 * den_y % P
    t = x * y % P
    if not ok or is_neg(t) or y == 0:
        return None
    return (x, y, 1, t)


def _map(t):
    r = SQRT_M1 * t % P * t % P
    u = (r + 1) * ONE_MINUS_D_SQ % P
    v = (-1 - r * D) * (r + D) % P
    was_square, s = sqrt_ratio_m1(u, v)
    s_prime = (-ct_abs(s * t)) % P
    if not was_square:
        s = s_prime
    c = P - 1 if was_square else r
    n = (c * (r - 1) % P * D_MINUS_ONE_SQ - v) % P
    w0 = 2 * s * v % P
    w1 = n * SQRT_AD_MINUS_ONE % P
    w2 = (1 - s * s) % P
    w3 = (1 + s * s) % P
    return (w0 * w3 % P, w2 * w1 % P, w1 * w3 % P, w0 * w2 % P)


def from_uniform(b64):
    r0 = int.from_bytes(b64[:32], "little") & ((1 << 255) - 1)
    r1 = int.from_bytes(b64[32:], "little") & ((1 << 255) - 1)
    return add(_map(r0 % P), _map(r1 % P))


def _base():
    y = 4 * inv(5) % P
    x = _sqrt((y * y - 1) * inv(D * y * y + 1))
    if is_neg(x):
        x = P - x
    return (x, y, 1, x * y % P)


G = _base()

# ── Hashing (RFC 9380) ──────────────────────────────────────────────────────


def xmd_sha512_64(msg, dst):
    dst_prime = dst + bytes([len(dst)])
    b0 = hashlib.sha512(bytes(128) + msg + (64).to_bytes(2, "big") + b"\x00" + dst_prime).digest()
    b1 = hashlib.sha512(b0 + b"\x01" + dst_prime).digest()
    return b1


def hash_to_group(dst, msg):
    return from_uniform(xmd_sha512_64(msg, dst))


def wide(b):
    return int.from_bytes(b, "little") % L


def sc_bytes(s):
    return (s % L).to_bytes(32, "little")


def u32(v):
    return v.to_bytes(4, "little")


def u64(v):
    return v.to_bytes(8, "little")

# ── linkring/1 ──────────────────────────────────────────────────────────────


N_RADIX = 16


def digits(n):
    m, cap = 1, N_RADIX
    while cap < n:
        m, cap = m + 1, cap * N_RADIX
    return m


def secret_from_seed(seed):
    return wide(hashlib.sha512(b"linkring/1:secret" + seed).digest())


def gens(m):
    return [hash_to_group(b"linkring/1:gen", u32(i)) for i in range(m * N_RADIX)]


def scope_base(scope):
    return hash_to_group(b"linkring/1:scope", scope)


def ring_digest(keys):
    return hashlib.sha256(b"".join(keys)).digest()


def challenge(n, digest, m, scope, tag, msg, pts):
    h = hashlib.sha512()
    h.update(b"linkring/1" + u32(N_RADIX) + u32(m) + u64(n) + digest)
    h.update(u64(len(scope)) + scope + tag + u64(len(msg)) + msg)
    for p in pts:
        h.update(encode(p))
    return wide(h.digest())


def msm(pairs):
    acc = IDENT
    for s, p in pairs:
        acc = add(acc, mul(s, p))
    return acc


def poly_mul_linear(poly, c1, c0):
    # poly · (c1·x + c0), coefficients low to high.
    out = [0] * (len(poly) + 1)
    for k, a in enumerate(poly):
        out[k] = (out[k] + a * c0) % L
        out[k + 1] = (out[k + 1] + a * c1) % L
    return out


def sign(keys, secret, l, scope, msg, aux):
    n = len(keys)
    m = digits(n)
    pts = [decode(k) for k in keys]
    digest = ring_digest(keys)
    u = scope_base(scope)
    tau = mul(secret, u)
    tag = encode(tau)
    hs = gens(m)
    seed = hashlib.sha512(b"linkring/1:nonce" + sc_bytes(secret) + digest + u64(n) + u64(l)
                          + u64(len(scope)) + scope + u64(len(msg)) + msg + u64(len(aux)) + aux).digest()
    ctr = [0]

    def nxt():
        v = wide(hashlib.sha512(seed + u32(ctr[0])).digest())
        ctr[0] += 1
        return v

    ld = [(l >> (4 * j)) & 15 for j in range(m)]
    a = [[0] * N_RADIX for _ in range(m)]
    for j in range(m):
        for i in range(1, N_RADIX):
            a[j][i] = nxt()
        a[j][0] = (-sum(a[j][1:])) % L
    r_a, r_b, r_c, r_d = nxt(), nxt(), nxt(), nxt()
    rho = [nxt() for _ in range(m)]
    sig = [[1 if ld[j] == i else 0 for i in range(N_RADIX)] for j in range(m)]

    def com(r, vals):
        return msm([(r, G)] + [(vals[j][i], hs[j * N_RADIX + i]) for j in range(m) for i in range(N_RADIX)])

    A = com(r_a, a)
    B = com(r_b, sig)
    C = com(r_c, [[a[j][i] * (1 - 2 * sig[j][i]) for i in range(N_RADIX)] for j in range(m)])
    Dc = com(r_d, [[-a[j][i] * a[j][i] for i in range(N_RADIX)] for j in range(m)])

    # Plain expansion over every padded entry; entry i ≥ n is keys[n-1].
    coef = [[0] * m for _ in range(n)]
    for i in range(N_RADIX ** m):
        poly = [1]
        for j in range(m):
            d = (i >> (4 * j)) & 15
            poly = poly_mul_linear(poly, 1 if d == ld[j] else 0, a[j][d])
        tgt = min(i, n - 1)
        for k in range(m):
            coef[tgt][k] = (coef[tgt][k] + poly[k]) % L
        if i == l:
            assert poly[m] == 1
        else:
            assert poly[m] == 0
    Gk = [msm([(coef[i][k], pts[i]) for i in range(n)] + [(rho[k], G)]) for k in range(m)]
    Yk = [mul(rho[k], u) for k in range(m)]
    x = challenge(n, digest, m, scope, tag, msg, [A, B, C, Dc] + Gk + Yk)
    body = bytes([m]) + b"".join(encode(p) for p in [A, B, C, Dc] + Gk + Yk)
    for j in range(m):
        for i in range(1, N_RADIX):
            body += sc_bytes(sig[j][i] * x + a[j][i])
    z = secret * pow(x, m, L) - sum(rho[k] * pow(x, k, L) for k in range(m))
    body += sc_bytes(r_b * x + r_a) + sc_bytes(r_c * x + r_d) + sc_bytes(z)
    return tag, body


def verify(keys, scope, msg, tag, body):
    n = len(keys)
    m = digits(n)
    if len(body) != 1 + 32 * (7 + 17 * m) or body[0] != m:
        return False
    pts = [decode(k) for k in keys]
    if any(p is None or is_ident(p) for p in pts):
        return False
    tau = decode(tag)
    if tau is None or is_ident(tau):
        return False
    off = 1
    cp = []
    for _ in range(4 + 2 * m):
        p = decode(body[off:off + 32])
        if p is None:
            return False
        cp.append(p)
        off += 32
    sv = []
    for _ in range(m * 15 + 3):
        s = int.from_bytes(body[off:off + 32], "little")
        if s >= L:
            return False
        sv.append(s)
        off += 32
    A, B, C, Dc = cp[:4]
    Gk, Yk = cp[4:4 + m], cp[4 + m:]
    z_a, z_c, z = sv[m * 15:]
    x = challenge(n, ring_digest(keys), m, scope, tag, msg, cp)
    f = [[0] * N_RADIX for _ in range(m)]
    for j in range(m):
        for i in range(1, N_RADIX):
            f[j][i] = sv[j * 15 + i - 1]
        f[j][0] = (x - sum(f[j][1:])) % L
    hs = gens(m)

    def com(r, vals):
        return msm([(r, G)] + [(vals[j][i], hs[j * N_RADIX + i]) for j in range(m) for i in range(N_RADIX)])

    if not eq(add(A, mul(x, B)), com(z_a, f)):
        return False
    if not eq(add(mul(x, C), Dc), com(z_c, [[f[j][i] * (x - f[j][i]) for i in range(N_RADIX)] for j in range(m)])):
        return False
    u = scope_base(scope)
    lhs = mul(pow(x, m, L), tau)
    for k in range(m):
        lhs = add(lhs, neg(mul(pow(x, k, L), Yk[k])))
    if not eq(lhs, mul(z, u)):
        return False
    per = [0] * n
    for i in range(N_RADIX ** m):
        p = 1
        for j in range(m):
            p = p * f[j][(i >> (4 * j)) & 15] % L
        per[min(i, n - 1)] = (per[min(i, n - 1)] + p) % L
    lhs = msm([(per[i], pts[i]) for i in range(n)])
    for k in range(m):
        lhs = add(lhs, neg(mul(pow(x, k, L), Gk[k])))
    return eq(lhs, mul(z, G))

# ── Self-test (RFC 9496 A.1 published vectors) ──────────────────────────────


RFC9496_MULTIPLES = [
    "0000000000000000000000000000000000000000000000000000000000000000",
    "e2f2ae0a6abc4e71a884a961c500515f58e30b6aa582dd8db6a65945e08d2d76",
    "6a493210f7499cd17fecb510ae0cea23a110e8d5b901f8acadd3095c73a3b919",
    "94741f5d5d52755ece4f23f044ee27d5d1ea1e2bd196b462166b16152a9d0259",
    "da80862773358b466ffadfe0b3293ab3d9fd53c5ea6c955358f568322daf6a57",
    "e882b131016b52c1d3337080187cf768423efccbb517bb495ab812c4160ff44e",
    "f64746d3c92b13050ed8d80236a7f0007c3b3f962f5ba793d19a601ebb1df403",
    "44f53520926ec81fbd5a387845beb7df85a96a24ece18738bdcfa6a7822a176d",
    "903293d8f2287ebe10e2374dc1a53e0bc887e592699f02d077d5263cdd55601c",
    "02622ace8f7303a31cafc63f8fc48fdc16e1c8c8d234b2f0d6685282a9076031",
    "20706fd788b2720a1ed2a5dad4952b01f413bcf0e7564de8cdc816689e2db95f",
    "bce83f8ba5dd2fa572864c24ba1810f9522bc6004afe95877ac73241cafdab42",
    "e4549ee16b9aa03099ca208c67adafcafa4c3f3e4e5303de6026e3ca8ff84460",
    "aa52e000df2e16f55fb1032fc33bc42742dad6bd5a8fc0be0167436c5948501f",
    "46376b80f409b29dc2b5f6f0c52591990896e5716f41477cd30085ab7f10301e",
    "e0c418f7c8d9c4cdd7395b93ea124f3ad99021bb681dfc3302a9d99a2e53e64e",
]


def selftest():
    acc = IDENT
    for i, want in enumerate(RFC9496_MULTIPLES):
        got = encode(acc).hex()
        if got != want:
            print(f"FAIL multiple {i}: {got} != {want}")
            return 1
        rt = decode(bytes.fromhex(want))
        if rt is None or not eq(rt, acc):
            print(f"FAIL decode {i}")
            return 1
        acc = add(acc, G)
    print("selftest ok")
    return 0

# ── Block I/O ───────────────────────────────────────────────────────────────


def parse_blocks(text):
    cases, cur = [], None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, _, val = line.partition(" ")
        if key == "case":
            cur = {"case": val}
        elif key == "end":
            cases.append(cur)
            cur = None
        else:
            cur[key] = val
    return cases


def hx(v):
    return bytes.fromhex(v) if v else b""


def do_sign(c):
    seeds = [hx(s) for s in c["seeds"].split(",")]
    secrets = [secret_from_seed(s) for s in seeds]
    keys = [encode(mul(s, G)) for s in secrets]
    l = int(c["signer"])
    tag, body = sign(keys, secrets[l], l, hx(c.get("scope", "")), hx(c.get("msg", "")), hx(c.get("aux", "")))
    return keys, tag, body


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "selftest"
    if mode == "selftest":
        return selftest()
    if selftest() != 0:
        return 1
    if mode == "sign":
        for c in parse_blocks(sys.stdin.read()):
            keys, tag, body = do_sign(c)
            print(f"case {c['case']}")
            print(f"ring {b''.join(keys).hex()}")
            print(f"digest {ring_digest(keys).hex()}")
            print(f"tag {tag.hex()}")
            print(f"body {body.hex()}")
            print("end")
        return 0
    if mode == "verify":
        for c in parse_blocks(sys.stdin.read()):
            ring = hx(c["ring"])
            keys = [ring[i:i + 32] for i in range(0, len(ring), 32)]
            ok = verify(keys, hx(c.get("scope", "")), hx(c.get("msg", "")), hx(c["tag"]), hx(c["body"]))
            print(f"case {c['case']}")
            print(f"ok {'true' if ok else 'false'}")
            print("end")
        return 0
    print(f"unknown mode {mode}")
    return 2


if __name__ == "__main__":
    sys.exit(main())
