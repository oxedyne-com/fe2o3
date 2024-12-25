#[macro_export]
/// Rust won't allow functions like pke_keygen to be shifted into the trait SaberAlgorithm
/// because they use associated constants, which can be changed by each implementation.  So we
/// have these implementation-specific wrappers, they are identical in each implementation.
macro_rules! generic_saber_api {
    () => (
        pub fn pke_keygen(
            &self,
            seed_a: [u8; SEED_BYTES],
            seed_s: [u8; NOISE_SEED_BYTES],
        ) -> (
            PublicKey<{Self::PK_LEN}>,
            SecretKeyCPA<{Self::SK_LEN}>,
        ) {
            Self::generic_pke_keygen::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
            >(
                &self,
                seed_a,
                seed_s,
            )
        }

        pub fn pke_enc(
            &self,
            m:      &[u8; KEY_BYTES],
            seed_sp:&[u8; SEED_BYTES],
            pk:     &PublicKey<{Self::POLY_VEC_COMPRESSED_BYTES}>,
        ) 
            -> CipherText<
                {Self::SCALE_BYTES_KEM},
                {Self::POLY_VEC_COMPRESSED_BYTES},
            >
        {
            Self::generic_pke_enc::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
                {Self::SCALE_BYTES_KEM},
            >(
                &self,
                m,
                seed_sp,
                pk,
            )
        }

        pub fn pke_dec(
            &self,
            ciphertext: &CipherText<
                {Self::SCALE_BYTES_KEM},
                {Self::POLY_VEC_COMPRESSED_BYTES},
            >,
            sk: &SecretKeyCPA<{Self::POLY_VEC_BYTES}>,
        )
            -> [u8; KEY_BYTES]
        {
            Self::generic_pke_dec::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
                {Self::SCALE_BYTES_KEM},
            >(
                &self,
                ciphertext,
                sk,
            )
        }

        pub fn kem_keygen(
            &self,
        ) -> (
            PublicKey<{Self::PK_LEN}>,
            SecretKeyCCA<{Self::SK_LEN}, {Self::PK_LEN}>,
        ) {
            let mut seed_a = [0_u8; SEED_BYTES];
            OsRng.fill_bytes(&mut seed_a);
            let mut seed_s = [0_u8; NOISE_SEED_BYTES];
            OsRng.fill_bytes(&mut seed_s);
            let mut rand = [0_u8; KEY_BYTES];
            OsRng.fill_bytes(&mut rand);

            Self::generic_kem_keygen::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
            >(
                &self,
                seed_a,
                seed_s,
                rand,
            )
        }

        pub fn kem_keygen_test(
            &self,
            seed_a: [u8; SEED_BYTES],
            seed_s: [u8; NOISE_SEED_BYTES],
            rand:   [u8; KEY_BYTES],
        ) -> (
            PublicKey<{Self::PK_LEN}>,
            SecretKeyCCA<{Self::SK_LEN}, {Self::PK_LEN}>,
        ) {
            Self::generic_kem_keygen::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
            >(
                &self,
                seed_a,
                seed_s,
                rand,
            )
        }

        pub fn kem_encap(
            &self,
            pk: &PublicKey<{Self::POLY_VEC_COMPRESSED_BYTES}>,
        ) -> (
            [u8; KEY_BYTES], // Session key
            CipherText<{Self::SCALE_BYTES_KEM}, {Self::POLY_VEC_COMPRESSED_BYTES}>,
        ) {
            let mut m = [0_u8; KEY_BYTES];
            OsRng.fill_bytes(&mut m);

            Self::generic_kem_encap::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
                {Self::SCALE_BYTES_KEM},
                {Self::CIPHERTEXT_BYTES},
            >(
                &self,
                pk,
                m,
            )
        }

        fn kem_encap_test(
            &self,
            pk: &PublicKey<{Self::POLY_VEC_COMPRESSED_BYTES}>,
            m: [u8; KEY_BYTES],
        ) -> (
            [u8; KEY_BYTES], // Session key
            CipherText<{Self::SCALE_BYTES_KEM}, {Self::POLY_VEC_COMPRESSED_BYTES}>,
        ) {
            Self::generic_kem_encap::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
                {Self::SCALE_BYTES_KEM},
                {Self::CIPHERTEXT_BYTES},
            >(
                &self,
                pk,
                m,
            )
        }

        pub fn kem_decap(
            &self,
            ct_bytes:   &[u8],
            sk:         &SecretKeyCCA<{Self::SK_LEN}, {Self::PK_LEN}>,
        )
            -> Outcome<[u8; KEY_BYTES]> // Session key
        {
            Self::generic_kem_decap::<
                {Self::L},
                {Self::POLY_VEC_BYTES},
                {Self::L * Self::POLY_VEC_BYTES},
                {Self::POLY_VEC_COMPRESSED_BYTES},
                {Self::POLY_COIN_BYTES},
                {Self::L * Self::POLY_COIN_BYTES},
                {Self::SCALE_BYTES_KEM},
                {Self::CIPHERTEXT_BYTES},
            >(
                &self,
                ct_bytes,
                sk,
            )
        }
    );
}

