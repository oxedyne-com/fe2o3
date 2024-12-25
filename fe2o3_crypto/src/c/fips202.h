#ifndef FIPS202_H
#define FIPS202_H

#include <stdint.h>

#define SHAKE128_RATE 168
#define SHAKE256_RATE 136
#define SHA3_256_RATE 136
#define SHA3_512_RATE 72

void saber_shake128(unsigned char *output, unsigned long long outlen, const unsigned char *input, unsigned long long inlen);
void saber_sha3_256(unsigned char *output, const unsigned char *input, unsigned long long inlen);
void saber_sha3_512(unsigned char *output, const unsigned char *input, unsigned long long inlen);

#endif
