use oxedize_fe2o3_core::{
    prelude::*,
    path::{
        self,
        NormalPath,
    },
    test::test_it,
};

use std::path::Path;

pub fn test_path(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Normalise path 000", "all", "path"], || {
        let cases = [
            ("./",                             (".", false)                 ),                
            ("../../",                         ("../..", true)              ),             
            ("/a/b/../c",                      ("./a/c", false)             ),            
            ("/a/b/../../c",                   ("./c", false)               ),            
            ("/a/b/../../../c",                ("../c", true)               ),            
            ("a/../../..",                     ("../..", true)              ),             
            ("./folder/../test.txt",           ("./test.txt", false)        ),       
            ("./../parent/folder",             ("../parent/folder", true)   ),  
            ("./././file",                     ("./file", false)            ),           
            (".././../file",                   ("../../file", true)         ),        
            ("folder/../../another",           ("../another", true)         ),        
            ("./folder/./file",                ("./folder/file", false)     ),    
            ("folder/subfolder/../../file",    ("./file", false)            ),           
            ("folder/../",                     (".", false)                 ),                
            ("folder/./subfolder/../file.txt", ("./folder/file.txt", false) ),
        ];
        for (i, (input, (expected, escapes))) in cases.iter().enumerate() {
            test!("Path case {} input: '{}'", i+1 , input);
            let path = Path::new(input).normalise();
            test!(" normalised: {:?}", path);
            req!(*escapes, path.escapes(), "(L: expected)");
            let result = path.into_inner().into_os_string().into_string().ok();
            test!(" string: {:?}", result);
            match result {
                Some(pstr) => req!(expected, &pstr),
                None => return Err(err!(
                    "Normalising the path '{}' produced {:?}.", input, result;
                Test, Mismatch)),
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Remove relative components of normalised path 000", "all", "path"], || {
        let cases = [
            ("./a",         "a"     ),  
            ("./a/b",       "a/b"   ),
            ("../a",        "a"     ),  
            ("../a/b/",     "a/b"   ),
            ("../../../a",  "a"     ),  
        ];
        for (i, (input, expected)) in cases.iter().enumerate() {
            test!("Path case {} input: {}", i+1, input);
            let path = Path::new(input).normalise().remove_relative();
            match path.into_inner().into_os_string().into_string() {
                Ok(s) => {
                    test!("result: {}", s);
                    req!(expected, &s, "(L: expected)");
                },
                Err(oss) => return Err(err!(
                    "Could not convert '{:?}' to string.", oss;
                Test, String, Conversion)),
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Absolute normalised path 000", "all", "path"], || {
        let cases = [
            ("./a",                 "/a"        ),  
            ("./a/b",               "/a/b"      ),
            ("../a",                "/a"        ),  
            ("../a/b/",             "/a/b"      ),
            ("../../../a",          "/a"        ),  
            ("/a/b/c/d/../../e",    "/a/b/e"    ),  
        ];
        for (i, (input, expected)) in cases.iter().enumerate() {
            test!("Path case {} input: {}", i+1, input);
            let path = Path::new(input).normalise().absolute();
            match path.into_inner().into_os_string().into_string() {
                Ok(s) => {
                    test!("result: {}", s);
                    req!(expected, &s, "(L: expected)");
                },
                Err(oss) => return Err(err!(
                    "Could not convert '{:?}' to string.", oss;
                Test, String, Conversion)),
            }
        }
        Ok(())
    }));

    res!(test_it(filter, &["Filename identifier", "all", "path"], || {
        let cases = [
            ("test.html",           true,       ),  
            ("./test.htm",          false       ),
            ("test/",               false       ),
            ("/test/",              false       ),
            ("../test/",            false       ),
            (".test.htm",           true        ),
        ];
        for (i, (input, expected)) in cases.iter().enumerate() {
            test!("Path case {} input: {}", i+1, input);
            req!(*expected, path::is_filename(input), "(L: expected)");
        }
        Ok(())
    }));

    Ok(())
}
