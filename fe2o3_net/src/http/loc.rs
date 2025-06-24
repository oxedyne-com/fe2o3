/// HTTP URL locator parsing and manipulation.
///
/// This module handles parsing and manipulating HTTP URL locators, which consist of:
/// - A path component
/// - Optional query parameters
/// - Optional fragment identifier
///
/// The locator follows the standard URL format:
/// `path?param1=value1&param2=value2#fragment`
///
/// # Examples
/// ```
/// use crate::HttpLocator;
///
/// let loc = HttpLocator::new("/path?key=value#section").unwrap();
/// assert_eq!(loc.path.as_str(), "/path");
/// assert!(loc.data.contains_key("key")); 
/// assert_eq!(loc.frag, "section");
/// ```
use crate::file::RequestPath;
use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;

use std::fmt;


/// Represents a parsed HTTP URL locator with path, query params, and fragment.
///
/// # Fields
/// * `path` - The validated request path component
/// * `data` - Map of parsed query parameters
/// * `frag` - Fragment identifier (part after #)
#[derive(Clone, Debug, Default)]
pub struct HttpLocator {
    pub path: RequestPath,
    pub data: DaticleMap,
    pub frag: String,
}

impl fmt::Display for HttpLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Write the path
        write!(f, "{}", self.path.as_str())?;

        if !self.data.is_empty() {
            write!(f, "?")?;
            for (i, (k, v)) in self.data.iter().enumerate() {
                write!(f, "{}={}", k, v)?;
                if i < self.data.len() - 1 {
                    write!(f, "&")?;
                }
            }
        }

        if !self.frag.is_empty() {
            write!(f, "#{}", self.frag)?;
        }

        Ok(())
    }
}

impl HttpLocator {

    pub fn new<S: Into<String>>(loc: S) -> Outcome<Self> {
        let (path, data, frag) = res!(Self::parse_locator_string(&loc.into()));
        Ok(Self {
            path,
            data,
            frag,
        })
    }

    fn parse_locator_string(path: &str) -> Outcome<(RequestPath, DaticleMap, String)> {
        let mut split = path.split('?');
        let path = RequestPath::new(split.next().unwrap_or_default().to_string());
        let rest = split.next().unwrap_or_default();
        let mut split_rest = rest.split('#');
        let query_string = split_rest.next().unwrap_or_default();
        let fragment = split_rest.next().unwrap_or_default();
        let map = res!(Self::parse_query_string(query_string));

        Ok((path, map, fragment.to_string()))
    }

    fn parse_query_string(query: &str) -> Outcome<DaticleMap> {
        let mut result = DaticleMap::new(); 
        for pair in query.split('&') {
            let mut split = pair.split('=');
            if let (Some(k), Some(v)) = (split.next(), split.next()) {
                let k = res!(Dat::from_str(k));
                let v = res!(Dat::from_str(v));
                result.insert(k, v);
            }
        }
        Ok(result)
        //Ok(query.split('&')
        //    .filter_map(|pair| {
        //        let mut split = pair.split('=');
        //        if let (Some(k), Some(v)) = (split.next(), split.next()) {
        //            let k = res!(Dat::from_str(k));
        //            let v = res!(Dat::from_str(v));
        //            Some((k, v))
        //        } else {
        //            None
        //        }
        //    })
        //    .collect()
        //)
    }
    
}
