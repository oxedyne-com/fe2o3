use crate::enc::saber::{
    self,
    LightSaber,
    Saber,
    FireSaber,
    SaberAlgorithm,
};
use console_error_panic_hook;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub fn greet(name: &str) {
    alert(&format!("Hello, {}!", name));
}

#[wasm_bindgen]
pub fn key_encap(
    scheme_id:      u8,
    pk_key:         &[u8],
    pk_seed:        &[u8],
    mut secret:     &mut [u8],
) ->
    Vec<u8> // Ciphertext
{
    console_error_panic_hook::set_once();
    match scheme_id {
        saber::LIGHTSABER_ID => {
            let scheme = saber::LightSaber;
            return scheme.generic_kem_encap_wasm::<
                {LightSaber::L},
                {LightSaber::POLY_VEC_BYTES},
                {LightSaber::L * LightSaber::POLY_VEC_BYTES},
                {LightSaber::POLY_VEC_COMPRESSED_BYTES},
                {LightSaber::POLY_COIN_BYTES},
                {LightSaber::L * LightSaber::POLY_COIN_BYTES},
                {LightSaber::SCALE_BYTES_KEM},
            >(
                pk_key,
                pk_seed,
                &mut secret,
            );
        },
        saber::SABER_ID => {
            let scheme = saber::Saber;
            return scheme.generic_kem_encap_wasm::<
                {Saber::L},
                {Saber::POLY_VEC_BYTES},
                {Saber::L * Saber::POLY_VEC_BYTES},
                {Saber::POLY_VEC_COMPRESSED_BYTES},
                {Saber::POLY_COIN_BYTES},
                {Saber::L * Saber::POLY_COIN_BYTES},
                {Saber::SCALE_BYTES_KEM},
            >(
                pk_key,
                pk_seed,
                &mut secret,
            );
        },
        saber::FIRESABER_ID => {
            let scheme = saber::FireSaber;
            return scheme.generic_kem_encap_wasm::<
                {FireSaber::L},
                {FireSaber::POLY_VEC_BYTES},
                {FireSaber::L * FireSaber::POLY_VEC_BYTES},
                {FireSaber::POLY_VEC_COMPRESSED_BYTES},
                {FireSaber::POLY_COIN_BYTES},
                {FireSaber::L * FireSaber::POLY_COIN_BYTES},
                {FireSaber::SCALE_BYTES_KEM},
            >(
                pk_key,
                pk_seed,
                &mut secret,
            );
        },
        _ => unimplemented!(),
    }
    
}
