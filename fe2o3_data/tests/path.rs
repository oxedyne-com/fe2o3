use oxedize_fe2o3_core::{
    prelude::*,
    path::NormalPath,
    test::test_it,
};

use std::path::Path;

pub fn test_path(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Normalise path 000", "all", "path"], || {
        let inputs = [
            "./",
            "../../",
            "/a/b/../c",
            "a/../../..",
            "./folder/../test.txt",             
            "./../parent/folder",               
            "./././file",                       
            ".././../file",                     
            "folder/../../another",             
            "./folder/./file",                  
            "folder/subfolder/../../file",      
            "folder/../",                       
            "folder/./subfolder/../file.txt",   
        ];
        let expected = [
            (".", false),
            ("../..", true),
            ("./a/c", false),
            ("../..", true),
            ("./test.txt", false),        
            ("../parent/folder", true),
            ("./file", false),            
            ("../../file", true),      
            ("../another", true),      
            ("./folder/file", false),     
            ("./file", false),            
            (".", false),               
            ("./folder/file.txt", false),  
        ];
        if inputs.len() != expected.len() {
            return Err(err!(
                "The number of inputs is {} which doesn't match the number of expected \
                outputs {}.", inputs.len(), expected.len();
            Test, Mismatch));
        }
        for (i, input) in inputs.iter().enumerate() {
            debug!("input: {}", input);
            let path = Path::new(input).normalise();
            debug!(" normalised: {:?}", path);
            req!(expected[i].1, path.escapes());
            let result = path.into_inner().into_os_string().into_string().ok();
            debug!(" string: {:?}", result);
            match result {
                Some(pstr) => req!(expected[i].0, &pstr),
                None => return Err(err!(
                    "Normalising the path '{}' produced {:?}.", input, result;
                Test, Mismatch)),
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Remove relative components of normalised path 000", "all", "path"], || {
        let inputs = [
            "./a",
            "./a/b",
            "../a",
            "../a/b/",
            "../../../a",
        ];
        let expected = [
            "a",
            "a/b",
            "a",
            "a/b",
            "a",
        ];
        if inputs.len() != expected.len() {
            return Err(err!(
                "The number of inputs is {} which doesn't match the number of expected \
                outputs {}.", inputs.len(), expected.len();
            Test, Mismatch));
        }
        for (i, input) in inputs.iter().enumerate() {
            debug!("input: {}", input);
            let path = Path::new(input).normalise().remove_relative();
            match path.into_inner().into_os_string().into_string() {
                Ok(s) => {
                    debug!("result: {}", s);
                    req!(expected[i], s);
                },
                Err(oss) => return Err(err!(
                    "Could not convert '{:?}' to string.", oss;
                Test, String, Conversion)),
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Absolute normalised path 000", "all", "path"], || {
        let inputs = [
            "./a",
            "./a/b",
            "../a",
            "../a/b/",
            "../../../a",
        ];
        let expected = [
            "/a",
            "/a/b",
            "/a",
            "/a/b",
            "/a",
        ];
        if inputs.len() != expected.len() {
            return Err(err!(
                "The number of inputs is {} which doesn't match the number of expected \
                outputs {}.", inputs.len(), expected.len();
            Test, Mismatch));
        }
        for (i, input) in inputs.iter().enumerate() {
            debug!("input: {}", input);
            let path = Path::new(input).normalise().absolute();
            match path.into_inner().into_os_string().into_string() {
                Ok(s) => {
                    debug!("result: {}", s);
                    req!(expected[i], s);
                },
                Err(oss) => return Err(err!(
                    "Could not convert '{:?}' to string.", oss;
                Test, String, Conversion)),
            }
        }
        Ok(())
    }));

    Ok(())
}
