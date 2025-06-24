use crate::id::NamexId;

use oxedyne_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    daticle::Vek,
    string::{
        dec::DecoderConfig,
        enc::{
            ByteEncoding,
            EncoderConfig,
        },
    },
    usr::{
        UsrKind,
        UsrKindCode,
        UsrKindId,
        UsrKinds,
    },
};
use oxedyne_fe2o3_text::string::Stringer;

use std::{
    collections::BTreeMap,
    fmt,
    io::Write,
    path::Path,
};


#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entity {
    // All lists maximum 10 members.
    pub nams:   Vec<String>,    // Preferred names.
    pub lang:   String,         // Language.
    pub desc:   Option<String>,         // A brief description (<= 100 words) of the entity.
    pub vers:   Option<String>,         // A string specifying the version.
    pub tags:   Option<Vec<String>>,    // Words or phrases helpful for categorisation (>= 1).
    pub tim1:   Option<String>,         // Origin date in Holocene Era format e.g. YYYYY-MM-DD.
    pub tim2:   Option<String>,         // End date in Holocene Era format e.g. YYYYY-MM-DD.
    pub refs:   Option<Vec<String>>,    // A list of references to more detailed information.
    pub alts:   Option<BTreeMap<String, String>>, // Alternative identifiers.
    pub lnks:   Option<BTreeMap<NamexId, String>>, // Relationships to other entities.
}

impl Entity {

    pub fn to_datmap<
        M1: MapMut<MapKey, Entity> + Clone + fmt::Debug + Default + Send,
        M2: MapMut<NamexId, Entity> + Clone + fmt::Debug + Default + Send,
    >(
        &self,
    )
        -> Outcome<Dat>
    {
        let mut map = BTreeMap::new();
        let ukinds = res!(Namex::<M1, M2>::ukinds());
        for (klab, ukid) in ukinds.label_to_id_map() {
            match klab.as_str() {
                "nams" => {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(100, Dat::Usr(ukid.clone(), None)),
                        dat!(self.nams.clone()),
                    );
                },
                "lang" => {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(200, Dat::Usr(ukid.clone(), None)),
                        dat!(self.lang.clone()),
                    );
                },
                "desc" => if let Some(desc) = &self.desc {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(300, Dat::Usr(ukid.clone(), None)),
                        dat!(desc.clone()),
                    );
                },
                "vers" => if let Some(vers) = &self.vers {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(350, Dat::Usr(ukid.clone(), None)),
                        dat!(vers.clone()),
                    );
                },
                "tim1" => if let Some(tim1) = &self.tim1 {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(400, Dat::Usr(ukid.clone(), None)),
                        dat!(tim1.clone()),
                    );
                },
                "tim2" => if let Some(tim2) = &self.tim2 {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(500, Dat::Usr(ukid.clone(), None)),
                        dat!(tim2.clone()),
                    );
                },
                "refs" => if let Some(refs) = &self.refs {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(600, Dat::Usr(ukid.clone(), None)),
                        dat!(refs.clone()),
                    );
                },
                "alts" => if let Some(alts) = &self.alts {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(650, Dat::Usr(ukid.clone(), None)),
                        {
                            let mut map = BTreeMap::new();
                            for (k, v) in alts {
                                map.insert(k.clone(), v.clone());
                            }
                            dat!(map)
                        }
                    );
                },
                "lnks" => if let Some(lnks) = &self.lnks {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(700, Dat::Usr(ukid.clone(), None)),
                        {
                            let mut map = BTreeMap::new();
                            for (k, v) in lnks {
                                map.insert(**k, v.clone());
                            }
                            dat!(map)
                        }
                    );
                },
                "tags" => if let Some(tags) = &self.tags {
                    map.insert(
                        oxedyne_fe2o3_jdat::map::MapKey::new(800, Dat::Usr(ukid.clone(), None)),
                        dat!(tags.clone()),
                    );
                },
                _ => (),
            }
        }
        Ok(Dat::OrdMap(map))
    }
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MapKey {
    ord: u64,
    id: NamexId,
}

impl MapKey {
    pub fn new(ord: u64, id: NamexId) -> Self {
        Self {
            ord,
            id,
        }
    }
    pub fn ord(&self) -> u64 { self.ord }
    pub fn id(&self) -> &NamexId { &self.id }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Namex<
    M1: MapMut<MapKey, Entity> + Clone + fmt::Debug + Default + Send,
    M2: MapMut<NamexId, Entity> + Clone + fmt::Debug + Default + Send,
> {
    ordered: M1,
    map: M2,
}

/// The Namex is distributed, meaning that users can store, manage and update what they regard
/// as the entire codex, or more likely, a part of it which we call a "tagset".  For better
/// efficiency, tagsets are subsets of the codex resulting from boolean searches of the "(tags)"
/// field.  Dividing the monolithic namex into smaller tagsets for specific purposes makes for
/// faster loading and search.
// 256 bits vs chars
// binary  32: -
// base64  44: 4IaH4F8elJw60EkIr2N9+S1avkvUDHX5IaH1GkEKoXQ=
// hex     64: e08687e05f1e949c3ad04908af637df92d5abe4bd40c75f921a1f51a410aa174
// decimal 78: 101555773374134092463836759906846316697927485927201504874204680125044748886388
impl<
    M1: MapMut<MapKey, Entity> + Clone + fmt::Debug + Default + Send,
    M2: MapMut<NamexId, Entity> + Clone + fmt::Debug + Default + Send,
>
    Namex<M1, M2>
{
    pub fn by_id(&self, id: &NamexId) -> Option<&Entity> {
        self.map.get(id)    
    }

    pub fn ukinds() -> Outcome<UsrKinds<
        BTreeMap<UsrKindCode, UsrKind>,
        BTreeMap<String, UsrKindId>
    >> {
        let mut uks = UsrKinds::new(BTreeMap::new(), BTreeMap::new());
        res!(uks.add(res!(Self::usr_key("nams"))));
        res!(uks.add(res!(Self::usr_key("lang"))));
        res!(uks.add(res!(Self::usr_key("desc"))));
        res!(uks.add(res!(Self::usr_key("vers"))));
        res!(uks.add(res!(Self::usr_key("tim1"))));
        res!(uks.add(res!(Self::usr_key("tim2"))));
        res!(uks.add(res!(Self::usr_key("refs"))));
        res!(uks.add(res!(Self::usr_key("alts"))));
        res!(uks.add(res!(Self::usr_key("lnks"))));
        res!(uks.add(res!(Self::usr_key("tags"))));
        Ok(uks)
    }

    /// The `UsrKindId` code establishes an order when these kinds are used for `Map` keys, but
    /// will be overridden by the order indices in `Entity::to_datmap` when forming an `OrdMap`.
    /// The codes here have been set to differ from the ordinals in `to_datmap` just to emphasise
    /// the difference.
    pub fn usr_key(label: &str) -> Outcome<UsrKindId> {
        Ok(match label {
            "nams"  => UsrKindId::new(10, Some("nams"), None),
            "lang"  => UsrKindId::new(20, Some("lang"), None),
            "desc"  => UsrKindId::new(30, Some("desc"), None),
            "vers"  => UsrKindId::new(35, Some("vers"), None),
            "tim1"  => UsrKindId::new(40, Some("tim1"), None),
            "tim2"  => UsrKindId::new(50, Some("tim2"), None),
            "refs"  => UsrKindId::new(60, Some("refs"), None),
            "alts"  => UsrKindId::new(65, Some("alts"), None),
            "lnks"  => UsrKindId::new(70, Some("lnks"), None),
            "tags"  => UsrKindId::new(80, Some("tags"), None),
            _ => return Err(err!(
                "Unrecognised usr key '{}'.", label;
            Input, Invalid, Unknown, Bug)),
        })
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Outcome<Self> {
        let s = res!(std::fs::read_to_string(path.as_ref()));
        let jdat_dec = DecoderConfig::<_, _>::jdat(Some(res!(Namex::<M1, M2>::ukinds())));
        let dat = res!(Dat::decode_string_with_config(s, &jdat_dec));
        if let Dat::OrdMap(map0) = dat {
            let mut map1 = M1::default();
            let mut map2 = M2::default();
            for (mk, vdat) in map0 {
                let kdat = mk.dat();
                let k = res!(NamexId::try_from(kdat));
                // Manually build Entity from datmap.
                let mut entity = Entity::default();
                match res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("nams")), None))),
                    &[&Kind::List, &Kind::Vek],
                )) {
                    Some(Dat::List(vecdat)) | Some(Dat::Vek(Vek(vecdat))) => {
                        let mut vecstr = Vec::new();
                        for d in vecdat {
                            let s = try_extract_dat!(d, Str);
                            vecstr.push(s.clone());
                        }
                        entity.nams = vecstr;
                    },
                    _ => return Err(err!(
                        "The key {:?} is missing required attribute 'nams'.", kdat;
                    Input, Missing)),
                }
                match res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("lang")), None))),
                    &[&Kind::Str],
                )) {
                    Some(Dat::Str(s)) => entity.lang = s.clone(),
                    _ => return Err(err!(
                        "The key {:?} is missing required attribute 'lang'.", kdat;
                    Input, Missing)),
                }
                if let Some(Dat::Str(s)) = res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("desc")), None))),
                    &[&Kind::Str],
                )) {
                    entity.desc = Some(s.clone());
                }
                if let Some(Dat::Str(s)) = res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("vers")), None))),
                    &[&Kind::Str],
                )) {
                    entity.vers = Some(s.clone());
                }
                match res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("tags")), None))),
                    &[&Kind::List, &Kind::Vek],
                )) {
                    Some(Dat::List(vecdat)) | Some(Dat::Vek(Vek(vecdat))) => {
                        let mut vecstr = Vec::new();
                        for d in vecdat {
                            let s = try_extract_dat!(d, Str);
                            vecstr.push(s.clone());
                        }
                        entity.tags = Some(vecstr);
                    },
                    _ => (),
                }
                if let Some(Dat::Str(s)) = res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("tim1")), None))),
                    &[&Kind::Str],
                )) {
                    entity.tim1 = Some(s.clone()); // TODO validate date
                }
                if let Some(Dat::Str(s)) = res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("tim2")), None))),
                    &[&Kind::Str],
                )) {
                    entity.tim2 = Some(s.clone()); // TODO validate date
                }
                match res!(vdat.map_get_type(&dat!("refs"), &[&Kind::List, &Kind::Vek])) {
                    Some(Dat::List(vecdat)) | Some(Dat::Vek(Vek(vecdat))) => {
                        let mut vecstr = Vec::new();
                        for d in vecdat {
                            let s = try_extract_dat!(d, Str);
                            vecstr.push(s.clone());
                        }
                        entity.tags = Some(vecstr);
                    },
                    _ => (),
                }
                match res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("refs")), None))),
                    &[&Kind::List, &Kind::Vek],
                )) {
                    Some(Dat::List(vecdat)) | Some(Dat::Vek(Vek(vecdat))) => {
                        let mut vecstr = Vec::new();
                        for d in vecdat {
                            let s = try_extract_dat!(d, Str);
                            vecstr.push(s.clone());
                        }
                        entity.refs = Some(vecstr);
                    },
                    _ => (),
                }
                match res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("alts")), None))),
                    &[&Kind::OrdMap],
                )) {
                    Some(Dat::OrdMap(datordmap)) => {
                        let mut altsmap = BTreeMap::new();
                        for (mapkey, vdat2) in datordmap {
                            let k2 = try_extract_dat!(mapkey.dat(), Str);
                            let v2 = try_extract_dat!(vdat2, Str);
                            altsmap.insert(k2.clone(), v2.clone());
                        }
                        entity.alts = Some(altsmap);
                    },
                    _ => (),
                }
                match res!(vdat.map_get_type(
                    &res!(Dat::try_from((res!(Self::usr_key("lnks")), None))),
                    &[&Kind::OrdMap],
                )) {
                    Some(Dat::OrdMap(datordmap)) => {
                        let mut linkmap = BTreeMap::new();
                        for (mapkey, vdat2) in datordmap {
                            let mkdat = res!(NamexId::try_from(mapkey.dat()));
                            let v2 = try_extract_dat!(vdat2, Str);
                            linkmap.insert(mkdat.clone(), v2.clone());
                        }
                        entity.lnks = Some(linkmap);
                    },
                    _ => (),
                }
                map1.insert(
                    MapKey {
                        ord: mk.ord(),
                        id: NamexId::from(k.clone()),
                    },
                    entity.clone(),
                );
                map2.insert(
                    NamexId::from(k),
                    entity,
                );
            }
            Ok(Namex {
                ordered: map1,
                map: map2,
            })
        } else {
            Err(err!(
                "Require a Daticle::Map, found a {}.", dat.kind();
            Invalid, Input))
        }
    }

    pub fn to_datmap(&self) -> Outcome<Dat> {
        let mut map = BTreeMap::new();
        for (umk, v) in self.ordered.iter() {
            map.insert(
                oxedyne_fe2o3_jdat::map::MapKey::new(umk.ord(), dat!(*umk.id().clone())),
                res!(v.to_datmap::<M1, M2>()),
            );    
        }
        Ok(Dat::OrdMap(map))
    }

    pub fn export_jdat(&self) -> Outcome<Vec<String>> {
        let dat = res!(self.to_datmap());
        let mut jdat_enc = EncoderConfig::<_, _>::jdat(Some(res!(Namex::<M1, M2>::ukinds())));
        jdat_enc.hide_usr_types = false;
        let jdatstr = res!(dat.encode_string_with_config(&jdat_enc));
        let lines = Stringer::new(jdatstr).to_lines("\t");
        Ok(lines)
    }

    pub fn export_json(&self) -> Outcome<Vec<String>> {
        let mut dat = res!(self.to_datmap());
        // Scrub all strings in the daticle of escape characters.
        res!(dat.normalise_string_values("  "));
        let mut json_enc = EncoderConfig::<_, _>::json(Some(res!(Namex::<M1, M2>::ukinds())));
        json_enc.byte_encoding = ByteEncoding::Base2x;
        let jsonstr = res!(dat.encode_string_with_config(&json_enc));
        let lines = Stringer::new(jsonstr).to_lines("\t");
        Ok(lines)
    }

    pub fn to_file(
        &self,
        lines: Vec<String>,
        path: &str,
    )
        -> Outcome<()>
    {
        let mut file = res!(std::fs::File::create(path));
        info!("Writing namex to file {}...", path);
        for mut line in lines {
            line.push_str("\n");
            res!(file.write(line.as_bytes()));
        }
        Ok(())
    }
}

//pub enum Relationship {
//    Inheritance,    // "is a"
//    Composition,    // "has a"
//    Aggregation,    // "is part of", "is contained in"
//    Ownership,      // "is owned by"
//    Control,        // "is controlled by"
//    Association,    // "is associated with"
//    Dependency,     // "depends on"
//    Creation,       // "was created by"
//    Sibling,        // "has the same creator as"
//}
