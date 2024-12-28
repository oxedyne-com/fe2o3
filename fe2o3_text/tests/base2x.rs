use oxedize_fe2o3_text::{
    base2x::{
        self,
        Base2x,
    },
    string::Stringer,
};

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};

use base64;
    

// Default constants.
const X_LEN: usize = 3;
const A_LEN: usize = base2x::alphabet_size(X_LEN as u32);

pub fn test_base2x(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Simple create 000", "all", "base2x"], || {
        const A_LEN: usize = 9;
        let alphabet = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_alphabet("ABCabc123"));
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("12_"));
        match Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))) {
            Ok(_base2x) => return Err(err!(
                "Alphabet of length {} should be invalid.", alphabet.len();
            Invalid, Input, Test)),
            Err(e) => msg!("Correctly triggered: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["Simple create 010", "all", "base2x"], || {
        let alphabet = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_alphabet("ABCabc12"));
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("12_"));
        match Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))) {
            Ok(_base2x) => (),
            Err(e) => return Err(e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["Simple create 020", "all", "base2x"], || {
        const A_LEN: usize = 300;
        let repeated_char = 'A';
        let repeat_count = 300;
        let alphabet: String =
            std::iter::repeat(repeated_char).take(repeat_count).collect();

        let alphabet = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_alphabet(&alphabet));
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("12_"));
        match Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))) {
            Ok(_base2x) => return Err(err!(
                "Alphabet of length {} should be invalid.", alphabet.len();
            Invalid, Input, Test)),
            Err(e) => test!("Correctly triggered: {}", e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["Simple create 030", "all", "base2x", "unicode"], || {
        let alphabet = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_alphabet("ABCab\u{e9}12"));
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("123"));
        match Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))) {
            Ok(_base2x) => (),
            Err(e) => return Err(e),
        };
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 000", "all", "base2x", "hematite"], || {
        let base2x = base2x::HEMATITE64;
        let a = base2x.alphabet_size();
        let x = base2x.token_size();
        let input = fmt!("This Hematite alphabet has {} characters", a);
        let input_byts = input.as_bytes();
        trace!("This is the alphabet we'll be using:");
        for line in base2x.fmt_char_map() {
            trace!(" {}", line);
        }
        test!("Let's start with the {} bytes of this text: '{}'", input_byts.len(), input);
        for byt in input_byts {
            debug!(" {:08b}", byt);
        }
        test!("Each of the {} tokens has a length of {} bits.", a, x);
        test!("Performing Base2x encoding from bytes to string...");
        let encoded = base2x.to_string(&input_byts);
        test!("Base2x encoding: '{}'", encoded);

        test!("Performing Base2x decoding from string to bytes...");
        let byts = res!(base2x.from_str(&encoded));
        let mut s = String::new();
        debug!("This gives us the following bytes:");
        for byt in &byts {
            debug!(" {:08b}", byt);
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        debug!("As a bit string: {}", g.insert_every("_", 8));
        debug!("As a bit string: {}", g.insert_every("_", x));

        let decoded = res!(std::str::from_utf8(&byts));
        test!("When the bytes are converted back to a string we get: '{}'", decoded);
        req!(decoded, input);
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 005", "all", "base2x", "hematite"], || {
        let base2x = base2x::HEMATITE32;
        let a = base2x.alphabet_size();
        let x = base2x.token_size();
        let input = fmt!("This Hematite alphabet has {} characters", a);
        let input_byts = input.as_bytes();
        test!("Let's start with the {} bytes of this text: '{}'", input_byts.len(), input);
        for byt in input_byts {
            debug!(" {:08b}", byt);
        }
        test!("Each of the {} tokens has a length of {} bits.", a, x);
        test!("Performing Base2x encoding from bytes to string...");
        let encoded = base2x.to_string(&input_byts);
        test!("Base2x encoding: '{}'", encoded);

        test!("Performing Base2x decoding from string to bytes...");
        debug!("This gives us the following bytes:");
        let byts = res!(base2x.from_str(&encoded));
        let mut s = String::new();
        for byt in &byts {
            debug!(" {:08b}", byt);
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        debug!("As a bit string: {}", g.insert_every("_", 8));
        debug!("As a bit string: {}", g.insert_every("_", x));

        let decoded = res!(std::str::from_utf8(&byts));
        test!("When the bytes are converted back to a string we get: '{}'", decoded);
        req!(decoded, input);
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 010", "all", "base2x"], || {
        let base2x = base2x::BASE64;
        let a = base2x.alphabet_size();
        let x = base2x.token_size();
        for line in base2x.fmt_char_map() {
            debug!("{}", line);
        }

        let input = "Man";
        test!("This is an example from the Wikipedia page for Base64.");
        let input_byts = input.as_bytes();
        test!("Let's start with the {} bytes of this text: '{}'", input_byts.len(), input);
        for byt in input_byts {
            test!(" {:08b}", byt);
        }
        test!("Each of the {} tokens has a length of {} bits.", a, x);
        test!("Performing Base2x encoding from bytes to string...");
        let encoded = base2x.to_string(&input_byts);
        test!("Base2x encoding: '{}'", encoded);

        let base64_encoded = base64::encode(&input_byts);
        test!("Now let's encode using the standard alphabet of Base64: '{}'", base64_encoded);
        test!("They should be identical, because there is no padding in this case.");
        req!(encoded, base64_encoded);
        test!("The character token mapping is:");
        for c in encoded.chars() {
            let token_opt = base2x.get_token(c);    
            test!(" '{}' -> {}", c,
                token_opt.map_or(
                    fmt!("None"),
                    |b| fmt!("{:0width$b}", b, width = x),
                ),
            );
        }

        test!("Performing Base2x decoding from string to bytes...");
        let byts = res!(base2x.from_str(&encoded));
        test!("This gives us the following bytes:");
        let mut s = String::new();
        for byt in &byts {
            test!(" {:08b}", byt);
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        test!("As a bit string: {}", g.insert_every("_", 8));
        test!("As a bit string: {}", g.insert_every("_", x));

        test!("Performing Base2x encoding back from bytes to string...");
        let decoded = res!(std::str::from_utf8(&byts));
        test!("When the bytes are converted back to a string we get: '{}'", decoded);
        req!(decoded, input);
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 011", "all", "base2x"], || {
        let base2x = base2x::BASE64;
        let a = base2x.alphabet_size();
        let x = base2x.token_size();
        for line in base2x.fmt_char_map() {
            debug!("{}", line);
        }

        let input = "Ma";
        test!("This is an example from the Wikipedia page for Base64.");
        let input_byts = input.as_bytes();
        test!("Let's start with the {} bytes of this text: '{}'", input_byts.len(), input);
        for byt in input_byts {
            test!(" {:08b}", byt);
        }
        test!("Each of the {} tokens has a length of {} bits.", a, x);
        test!("Performing Base2x encoding from bytes to string...");
        let encoded = base2x.to_string(&input_byts);
        test!("Base2x encoding: '{}'", encoded);
        let mut compare = encoded.clone();
        compare.pop();
        test!("This should include padding, so let's remove the Base2x-specific part: '{}'", compare);
        let base64_encoded = base64::encode(&input_byts);
        test!("The character token mapping is:");
        for c in encoded.chars() {
            let token_opt = base2x.get_token(c);    
            test!(" '{}' -> {}", c,
                token_opt.map_or(
                    fmt!("None"),
                    |b| fmt!("{:0width$b}", b, width = x),
                ),
            );
        }
        test!("Now let's encode using the standard alphabet of Base64: '{}'", base64_encoded);
        test!("They should be identical, because they use a similar padding scheme.");
        req!(compare, base64_encoded);

        test!("Performing Base2x decoding from string to bytes...");
        let byts = res!(base2x.from_str(&encoded));
        test!("This gives us the following bytes:");
        let mut s = String::new();
        for byt in &byts {
            test!(" {:08b}", byt);
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        test!("As a bit string: {}", g.insert_every("_", 8));
        test!("As a bit string: {}", g.insert_every("_", x));
        test!("You can see at the end here that the last character is partial, with only 4 bits.");
        test!("The padding appends 2 zero bits, giving 000100 or 'E'.");

        test!("Performing Base2x encoding back from bytes to string...");
        let decoded = res!(std::str::from_utf8(&byts));
        test!("When the bytes are converted back to a string we get: '{}'", decoded);
        req!(decoded, input);
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 100", "all", "base2x"], || {
        let alphabet = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_alphabet("ABCabc12"));
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("12_"));
        let base2x = res!(Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))));
        test!("This is the alphabet we'll be using:");
        for line in base2x.fmt_char_map() {
            test!(" {}", line);
        }
        let mut input = "BA2c1baBCaa21b111AcCb".to_string();
        test!("Let's start with an arbitrary Base2x encoding: '{}'", input);
        test!("Since each token is {} bits long, that's {} bits which is not divisible by 8.",
            X_LEN, input.len() * X_LEN);
        test!("Which means padding is needed, so let's apply it using Base2x normalisation...");
        let padding = base2x.normalise(&mut input);
        if padding > 0 {
            base2x.push_pad(&mut input, padding);
        }
        test!("Normalised: '{}' with {} bits of padding.", input, padding);
        test!("The token values for each encoded character are:");
        for c in input.chars() {
            let token_opt = base2x.get_token(c);    
            test!(" '{}' -> {}", c,
                token_opt.map_or(
                    fmt!("None"),
                    |b| fmt!("{:0width$b}", b, width = X_LEN),
                ),
            );
        }
        test!("Performing Base2x decoding from string to bytes...");
        let byts = res!(base2x.from_str(&input));
        test!("This gives us the following bytes:");
        let mut s = String::new();
        for byt in &byts {
            test!(" {:08b}", byt);
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        test!("As a bit string: {}", g.insert_every("_", 8));
        test!("As a bit string: {}", g.insert_every("_", X_LEN));

        test!("Performing Base2x encoding back from bytes to string...");
        let encoded = base2x.to_string(&byts);
        test!("When the bytes are Base2x encoded back to a string we get: '{}'", encoded);
        req!(encoded, input);
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 200", "all", "base2x"], || {
        let byts = vec![0b01001011, 0b11101000, 0b10110100, 0b01101111];
        test!("input bytes:");
        for byt in &byts {
            test!("{:08b}", byt);
        }
        let alphabet = "ABCabc12";
        test!("alphabet = '{}'", alphabet);
        let alphabet = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_alphabet(alphabet));
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("12_"));
        let base2x = res!(Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))));
        let n = byts.len();
        let n_bits = 8*n;
        let n_tokens = n_bits / base2x.token_size();
        let rem = n_bits - n_tokens * base2x.token_size();
        test!("{} input bytes have {} bits, requring {} tokens with a remainder of {}",
            n, n_bits, n_tokens, rem);
        let mut s = String::new();
        for byt in &byts {
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        test!("bit string: {}", g.insert_every("_", 8));
        test!("bit string: {}", g.insert_every("_", 3));
        let encoded = base2x.to_string(&byts);
        test!("encoded = '{}'", encoded);

        test!("encoded '{}' has {} characters, requring {} bits",
            encoded, encoded.len(), base2x.token_size() * encoded.len());
        for c in encoded.chars() {
            let token_opt = base2x.get_token(c);    
            test!(" '{}' -> {}", c,
                token_opt.map_or(
                    fmt!("None"),
                    |b| fmt!("{:0width$b}", b, width = base2x.token_size()),
                ),
            );
        }
        // Now decode string -> bytes
        let byts2 = res!(base2x.from_str(&encoded));
        let mut s = String::new();
        for byt in &byts2 {
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        test!("decoded bit string: {}", g.insert_every("_", 8));
        test!("decoded bit string: {}", g.insert_every("_", base2x.token_size()));
        req!(byts2, byts);
        Ok(())
    }));

    res!(test_it(filter, &["Round trip 300", "all", "base2x", "unicode"], || {
        const X_LEN: usize = 9;
        const A_LEN: usize = base2x::alphabet_size(X_LEN as u32);
        let pad_set = res!(Base2x::<{A_LEN}, {X_LEN}>::prepare_pad_set("12345678_"));
        let alphabet = [
            '\u{1f600}',
            '\u{1f976}',
            '\u{1f4a9}',
            '\u{1f63b}',
            '\u{1f44c}',
            '\u{1f441}',
            '\u{1f937}',
            '\u{1f3cb}',
            '\u{1f984}',
            '\u{1f30f}',
            '\u{26f5}', 
            '\u{2708}', 
            '\u{1fa90}',
            '\u{1fabf}',
            '\u{1f438}',
            '\u{1f40a}',
            '\u{1f422}',
            '\u{1f98e}',
            '\u{1f40d}',
            '\u{1f432}',
            '\u{1f409}',
            '\u{1f995}',
            '\u{1f996}',
            '\u{1f433}',
            '\u{1f40b}',
            '\u{1f42c}',
            '\u{1f9ad}',
            '\u{1f41f}',
            '\u{1f420}',
            '\u{1f421}',
            '\u{1f988}',
            '\u{1f419}',
            '\u{1f41a}',
            '\u{1fab8}',
            '\u{1fabc}',
            '\u{1f40c}',
            '\u{1f98b}',
            '\u{1f41b}',
            '\u{1f41c}',
            '\u{1f41d}',
            '\u{1fab2}',
            '\u{1f41e}',
            '\u{1f997}',
            '\u{1fab3}',
            '\u{1f577}',
            '\u{1f578}',
            '\u{1f982}',
            '\u{1f99f}',
            '\u{1fab0}',
            '\u{1fab1}',
            '\u{1f9a0}',
            '\u{1f490}',
            '\u{1f338}',
            '\u{1f4ae}',
            '\u{1fab7}',
            '\u{1f3f5}',
            '\u{1f339}',
            '\u{1f940}',
            '\u{1f33a}',
            '\u{1f33b}',
            '\u{1f33c}',
            '\u{1f337}',
            '\u{1fabb}',
            '\u{1f331}',
            '\u{1fab4}',
            '\u{1f332}',
            '\u{1f333}',
            '\u{1f334}',
            '\u{1f335}',
            '\u{1f33e}',
            '\u{1f33f}',
            '\u{2618}', 
            '\u{1f340}',
            '\u{1f341}',
            '\u{1f342}',
            '\u{1f343}',
            '\u{1fab9}',
            '\u{1faba}',
            '\u{1f344}',
            '\u{1f347}',
            '\u{1f348}',
            '\u{1f349}',
            '\u{1f34a}',
            '\u{1f34b}',
            '\u{1f34c}',
            '\u{1f34d}',
            '\u{1f96d}',
            '\u{1f34e}',
            '\u{1f34f}',
            '\u{1f350}',
            '\u{1f351}',
            '\u{1f352}',
            '\u{1f353}',
            '\u{1fad0}',
            '\u{1f95d}',
            '\u{1f345}',
            '\u{1fad2}',
            '\u{1f965}',
            '\u{1f951}',
            '\u{1f346}',
            '\u{1f954}',
            '\u{1f955}',
            '\u{1f33d}',
            '\u{1f336}',
            '\u{1fad1}',
            '\u{1f952}',
            '\u{1f96c}',
            '\u{1f966}',
            '\u{1f9c4}',
            '\u{1f9c5}',
            '\u{1f95c}',
            '\u{1fad8}',
            '\u{1f330}',
            '\u{1fada}',
            '\u{1fadb}',
            '\u{1f35e}',
            '\u{1f950}',
            '\u{1f956}',
            '\u{1fad3}',
            '\u{1f968}',
            '\u{1f96f}',
            '\u{1f95e}',
            '\u{1f9c7}',
            '\u{1f9c0}',
            '\u{1f356}',
            '\u{1f357}',
            '\u{1f969}',
            '\u{1f953}',
            '\u{1f354}',
            '\u{1f35f}',
            '\u{1f355}',
            '\u{1f32d}',
            '\u{1f96a}',
            '\u{1f32e}',
            '\u{1f32f}',
            '\u{1fad4}',
            '\u{1f959}',
            '\u{1f9c6}',
            '\u{1f95a}',
            '\u{1f373}',
            '\u{1f958}',
            '\u{1f372}',
            '\u{1fad5}',
            '\u{1f963}',
            '\u{1f957}',
            '\u{1f37f}',
            '\u{1f9c8}',
            '\u{1f9c2}',
            '\u{1f96b}',
            '\u{1f371}',
            '\u{1f358}',
            '\u{1f359}',
            '\u{1f35a}',
            '\u{1f35b}',
            '\u{1f35c}',
            '\u{1f35d}',
            '\u{1f360}',
            '\u{1f362}',
            '\u{1f363}',
            '\u{1f364}',
            '\u{1f365}',
            '\u{1f96e}',
            '\u{1f361}',
            '\u{1f95f}',
            '\u{1f960}',
            '\u{1f961}',
            '\u{1f980}',
            '\u{1f99e}',
            '\u{1f990}',
            '\u{1f991}',
            '\u{1f9aa}',
            '\u{1f366}',
            '\u{1f367}',
            '\u{1f368}',
            '\u{1f369}',
            '\u{1f36a}',
            '\u{1f382}',
            '\u{1f370}',
            '\u{1f9c1}',
            '\u{1f967}',
            '\u{1f36b}',
            '\u{1f36c}',
            '\u{1f36d}',
            '\u{1f36e}',
            '\u{1f36f}',
            '\u{1f37c}',
            '\u{1f95b}',
            '\u{2615}', 
            '\u{1fad6}',
            '\u{1f375}',
            '\u{1f376}',
            '\u{1f37e}',
            '\u{1f377}',
            '\u{1f378}',
            '\u{26f0}',
            '\u{1f30b}',
            '\u{1f5fb}',
            '\u{1f3d5}',
            '\u{1f3d6}',
            '\u{1f3dc}',
            '\u{1f3dd}',
            '\u{1f3de}',
            '\u{1f3df}',
            '\u{1f3db}',
            '\u{1f3d7}',
            '\u{1f9f1}',
            '\u{1faa8}',
            '\u{1fab5}',
            '\u{1f6d6}',
            '\u{1f3d8}',
            '\u{1f3da}',
            '\u{1f3e0}',
            '\u{1f3e1}',
            '\u{1f3e2}',
            '\u{1f3e3}',
            '\u{1f3e4}',
            '\u{1f3e5}',
            '\u{1f3e6}',
            '\u{1f3e8}',
            '\u{1f3e9}',
            '\u{1f3ea}',
            '\u{1f3eb}',
            '\u{1f3ec}',
            '\u{1f3ed}',
            '\u{1f3ef}',
            '\u{1f3f0}',
            '\u{1f492}',
            '\u{1f5fc}',
            '\u{1f5fd}',
            '\u{26ea}',
            '\u{1f54c}',
            '\u{1f6d5}',
            '\u{1f54d}',
            '\u{26e9}',
            '\u{1f54b}',
            '\u{26f2}',
            '\u{26fa}',
            '\u{1f301}',
            '\u{1f303}',
            '\u{1f3d9}',
            '\u{1f304}',
            '\u{1f305}',
            '\u{1f306}',
            '\u{1f307}',
            '\u{1f309}',
            '\u{2668}',
            '\u{1f3a0}',
            '\u{1f6dd}',
            '\u{1f3a1}',
            '\u{1f3a2}',
            '\u{1f488}',
            '\u{1f3aa}',
            '\u{1f682}',
            '\u{1f326}',
            '\u{1f327}',
            '\u{1f328}',
            '\u{1f329}',
            '\u{1f32a}',
            '\u{1f32b}',
            '\u{1f32c}',
            '\u{1f300}',
            '\u{1f308}',
            '\u{1f302}',
            '\u{2602}',
            '\u{2614}',
            '\u{26f1}',
            '\u{26a1}',
            '\u{2744}',
            '\u{2603}',
            '\u{26c4}',
            '\u{2604}',
            '\u{1f525}',
            '\u{1f4a7}',
            '\u{1f30a}',
            '\u{1f383}',
            '\u{1f384}',
            '\u{1f386}',
            '\u{1f387}',
            '\u{1f9e8}',
            '\u{2728}',
            '\u{1f388}',
            '\u{1f389}',
            '\u{1f38a}',
            '\u{1f38b}',
            '\u{1f38d}',
            '\u{1f38e}',
            '\u{1f38f}',
            '\u{1f390}',
            '\u{1f391}',
            '\u{1f9e7}',
            '\u{1f380}',
            '\u{1f381}',
            '\u{1f397}',
            '\u{1f39f}',
            '\u{1f3ab}',
            '\u{1f396}',
            '\u{1f3c6}',
            '\u{1f3c5}',
            '\u{1f947}',
            '\u{1f948}',
            '\u{1f949}',
            '\u{26bd}',
            '\u{26be}',
            '\u{1f94e}',
            '\u{1f3c0}',
            '\u{1f3d0}',
            '\u{1f3c8}',
            '\u{1f3c9}',
            '\u{1f3be}',
            '\u{1f94f}',
            '\u{1f3b3}',
            '\u{1f3cf}',
            '\u{1f3d1}',
            '\u{1f3d2}',
            '\u{1f94d}',
            '\u{1f3d3}',
            '\u{1f3f8}',
            '\u{1f94a}',
            '\u{1f94b}',
            '\u{1f945}',
            '\u{26f3}',
            '\u{26f8}',
            '\u{1f3a3}',
            '\u{1f93f}',
            '\u{1f3bd}',
            '\u{1f3bf}',
            '\u{1f6f7}',
            '\u{1f94c}',
            '\u{1f3af}',
            '\u{1fa80}',
            '\u{1fa81}',
            '\u{1f52b}',
            '\u{1f3b1}',
            '\u{1f52e}',
            '\u{1fa84}',
            '\u{1f3ae}',
            '\u{1f579}',
            '\u{1f3b0}',
            '\u{1f3b2}',
            '\u{1f9e9}',
            '\u{1f9f8}',
            '\u{1fa85}',
            '\u{1faa9}',
            '\u{1fa86}',
            '\u{2660}',
            '\u{2665}',
            '\u{2666}',
            '\u{2663}',
            '\u{265f}',
            '\u{1f0cf}',
            '\u{1f004}',
            '\u{1f3b4}',
            '\u{1f3ad}',
            '\u{1f5bc}',
            '\u{1f3a8}',
            '\u{1f9f5}',
            '\u{1faa1}',
            '\u{1f9f6}',
            '\u{1faa2}',
            '\u{1f453}',
            '\u{1f576}',
            '\u{1f97d}',
            '\u{1f97c}',
            '\u{1f9ba}',
            '\u{1f454}',
            '\u{1f455}',
            '\u{1f456}',
            '\u{1f9e3}',
            '\u{1f9e4}',
            '\u{1f9e5}',
            '\u{1f9e6}',
            '\u{1f457}',
            '\u{1f458}',
            '\u{1f97b}',
            '\u{1fa71}',
            '\u{1fa72}',
            '\u{1fa73}',
            '\u{1f459}',
            '\u{1f45a}',
            '\u{1faad}',
            '\u{1f45b}',
            '\u{1f45c}',
            '\u{1f45d}',
            '\u{1f6cd}',
            '\u{1f392}',
            '\u{1fa74}',
            '\u{1f45e}',
            '\u{1f45f}',
            '\u{1f97e}',
            '\u{1f97f}',
            '\u{1f460}',
            '\u{1f461}',
            '\u{1fa70}',
            '\u{1f462}',
            '\u{1faae}',
            '\u{1f451}',
            '\u{1f452}',
            '\u{1f3a9}',
            '\u{1f393}',
            '\u{1f9e2}',
            '\u{1fa96}',
            '\u{26d1}',
            '\u{1f4ff}',
            '\u{1f484}',
            '\u{1f48d}',
            '\u{1f48e}',
            '\u{1f507}',
            '\u{1f508}',
            '\u{1f509}',
            '\u{1f50a}',
            '\u{1f4e2}',
            '\u{1f4e3}',
            '\u{1f4ef}',
            '\u{1f514}',
            '\u{1f515}',
            '\u{1f3bc}',
            '\u{1f3b5}',
            '\u{1f3b6}',
            '\u{1f399}',
            '\u{1f39a}',
            '\u{1f39b}',
            '\u{1f3a4}',
            '\u{1f3a7}',
            '\u{1f4fb}',
            '\u{1f3b7}',
            '\u{1fa97}',
            '\u{1f3b8}',
            '\u{1f3b9}',
            '\u{1f3ba}',
            '\u{1f3bb}',
            '\u{1fa95}',
            '\u{1f941}',
            '\u{1fa98}',
            '\u{1fa87}',
            '\u{1fa88}',
            '\u{1f4f1}',
            '\u{1f4f2}',
            '\u{260e}',
            '\u{1f4de}',
            '\u{1f4df}',
            '\u{1f4e0}',
            '\u{1f50b}',
            '\u{1faab}',
            '\u{1f50c}',
            '\u{1f4bb}',
            '\u{1f5a5}',
            '\u{1f5a8}',
            '\u{2328}',
            '\u{1f5b1}',
            '\u{1f5b2}',
            '\u{1f4bd}',
            '\u{1f4be}',
            '\u{1f4bf}',
            '\u{1f4c0}',
            '\u{1f9ee}',
            '\u{1f3a5}',
            '\u{1f39e}',
            '\u{1f4fd}',
            '\u{1f3ac}',
            '\u{1f4fa}',
            '\u{1f4f7}',
            '\u{1f4f8}',
            '\u{1f4f9}',
            '\u{1f4fc}',
            '\u{1f50d}',
            '\u{1f50e}',
            '\u{1f56f}',
            '\u{1f4a1}',
            '\u{1f526}',
            '\u{1f3ee}',
            '\u{1fa94}',
            '\u{1f4d4}',
            '\u{1f4d5}',
            '\u{1f4d6}',
            '\u{1f4d7}',
            '\u{1f4d8}',
            '\u{1f4d9}',
            '\u{1f4da}',
            '\u{1f4d3}',
            '\u{1f4d2}',
            '\u{1f4c3}',
            '\u{1f4dc}',
            '\u{1f4c4}',
            '\u{1f4f0}',
            '\u{1f5de}',
            '\u{1f4d1}',
            '\u{1f516}',
            '\u{1f3f7}',
            '\u{1f4b0}',
            '\u{1fa99}',
            '\u{1f4b4}',
            '\u{1f4b5}',
            '\u{1f4b6}',
            '\u{1f4b7}',
            '\u{1f4b8}',
            '\u{1f4b3}',
            '\u{1f9fe}',
            '\u{1f4b9}',
            '\u{2709}',
            '\u{1f4e7}',
            '\u{1f4e8}',
            '\u{1f4e9}',
            '\u{1f4e4}',
            '\u{1f4e5}',
            '\u{1f4e6}',
            '\u{1f4eb}',
            '\u{1f4ea}',
            '\u{1f4ec}',
            '\u{1f4ed}',
            '\u{1f4ee}',
            '\u{1f5f3}',
            '\u{270f}',
        ];
        let base2x = res!(Base2x::<{A_LEN}, {X_LEN}>::new(alphabet, Some(('=', pad_set))));

        test!("Here we demonstrate a unicode alphabet of {} emojis with a token size of {}.",
            A_LEN, X_LEN);
        test!("Because the token size exceeds 8 bits, this yields some compression.");
        test!("Unicode only has 1,424 in 2024, so in order to represent, say unique ids with");
        test!(" just a few emojis, we'll need to create a larger, custom set of glyphs.");
        let first = 10;
        test!("Here are the first {} characters of the alphabet:", first);
        for token in 0..first {
            let c = base2x.get_char(token);    
            test!(" '{:0width$b}' -> {}", token, c, width = X_LEN);
        }         // 12345678901234567890123456789012
        let input = "Every moment is a new beginning.";
        let input_byts = input.as_bytes();
        test!("Let's start with the {} bytes of this text: '{}'", input_byts.len(), input);
        let mut s = String::new();
        for byt in input_byts {
            test!(" {:08b}", byt);
            s.push_str(&fmt!("{:08b}", byt));
        }
        let g = Stringer::new(s);
        test!("As a bit string: {}", g.insert_every("_", 8));
        test!("As a bit string: {}", g.insert_every("_", X_LEN));
        test!("Performing Base2x encoding from bytes to string...");
        let encoded = base2x.to_string(&input_byts[..]);
        test!("Base2x encoding: '{}'", encoded);
        test!("Performing Base2x decoding from string to bytes...");
        let byts = res!(base2x.from_str(&encoded));
        let decoded = res!(std::str::from_utf8(&byts));
        test!("When the bytes are converted back to a string we get: '{}'", decoded);
        req!(decoded, input);
        Ok(())
    }));

    Ok(())
}
