# Protomaps sample

Map data © OpenStreetMap contributors, available under the Open Database Licence
(https://www.openstreetmap.org/copyright), from the Protomaps daily planet build
`https://build.protomaps.com/20260922.pmtiles` (basemap v4.15.2).

The files are an oracle captured by the reference JavaScript readers -- `pmtiles` 4.5.0,
`@mapbox/vector-tile` 3.0.0 and `pbf` 5.1.2 -- on 2026-09-23:

- `ocean_ranges.bin`: every byte range the `pmtiles` reader read to fetch two open-ocean
  tiles, as `[u64 offset | u32 length | bytes]*`, little-endian: the header and root
  directory, their two leaf directories and the one tile content they share.
- `expect.json`: the archive header as that reader parsed it, fifty tiles with their Hilbert
  ids and the length and FNV-1a-64 of each decompressed tile, and ids converted both ways.
- `<z>_<x>_<y>.mvt` and `.json.gz`: two small tiles, decompressed, and the reference
  decoder's reading of them (layers, features, ids, types, properties, geometry).
- `sample.pmtiles`: a small archive written by the reference Python writer (`pmtiles`
  3.8.1) with `make_sample.py`, holding the two small tiles above and the open-ocean tile
  addressed as a run of three, all gzipped: a real archive not written by this library.
