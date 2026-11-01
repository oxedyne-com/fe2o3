#!/usr/bin/env python3
# Writes padme_vectors.txt, the vectors fe2o3_crypto's `pad` (tests/pad.rs) and its JS twin
# (js/test/pad.test.mjs) both check. It is an oracle independent of either: Algorithm 1 of
# Nikitin et al., "Reducing Metadata Leakage from Encrypted Files and Communication with PURBs",
# PoPETs 2019, arXiv 1806.03160, §4.4, written straight from the paper.
#
#   python3 -I gen_padme_vectors.py > padme_vectors.txt
import random

def padme(L):
    if L < 2:
        return L                      # floor(log2 L) is undefined below 2; the length stands
    E = L.bit_length() - 1            # floor(log2 L)
    S = E.bit_length()                # floor(log2 E) + 1
    z = E - S
    m = (1 << z) - 1
    return (L + m) & ~m

PREFIX = 4                            # u32 big-endian length before the body

def rule_len(rule, block, L):
    if rule == "padme":
        return padme(L)
    if rule == "pow2":
        return 1 if L <= 1 else 1 << (L - 1).bit_length()
    if rule == "block":
        return -(-L // block) * block
    raise ValueError(rule)

def frame(rule, floor, block, n):
    return max(rule_len(rule, block, PREFIX + n), floor)

rng = random.Random(1806_03160)
lens = list(range(0, 1100))
lens += [rng.randrange(1100, 1 << 20) for _ in range(2000)]
lens += [(1 << k) + d for k in range(10, 53) for d in (-1, 0, 1)]
lens += [rng.randrange(1 << 20, 1 << 52) for _ in range(200)]
print("# padme <L> <padded>: Algorithm 1, arXiv 1806.03160 §4.4.")
for L in sorted(set(lens)):
    print("padme", L, padme(L))
print("# frame <rule> <floor> <block> <body bytes> <framed bytes>: 4-byte length prefix, body, zeros.")
for (rule, floor, block) in [("padme", 0, 0), ("padme", 256, 0), ("pow2", 256, 0), ("block", 0, 1048576), ("block", 512, 4096)]:
    for n in sorted(set(list(range(0, 300)) + [rng.randrange(300, 1 << 22) for _ in range(300)])):
        print("frame", rule, floor, block, n, frame(rule, floor, block, n))
