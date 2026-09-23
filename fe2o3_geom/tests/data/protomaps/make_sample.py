# A small real PMTiles archive, written by the reference Python writer (pmtiles 3.8.1),
# holding real Protomaps tiles: two Perth tiles and one open-ocean content addressed as a run
# of three.  Tiles are stored gzipped, as the planet stores them.
import gzip, sys, json
sys.path.insert(0, 'site')
from pmtiles.writer import write
from pmtiles.tile import zxy_to_tileid, TileType, Compression

OUT = sys.argv[1]
SRC = sys.argv[2]
tiles = {}
for (z, x, y) in [(15, 26930, 19457), (13, 6729, 4865)]:
    tiles[zxy_to_tileid(z, x, y)] = gzip.compress(open(f'{SRC}/protomaps_{z}_{x}_{y}.mvt', 'rb').read(), mtime=0)
ocean = gzip.compress(open(f'{SRC}/protomaps_12_2958_2545.mvt', 'rb').read(), mtime=0)
base = zxy_to_tileid(12, 2958, 2545)
for k in range(3):
    tiles[base + k] = ocean
with write(OUT) as w:
    for tid in sorted(tiles):
        w.write_tile(tid, tiles[tid])
    w.finalize({
        'tile_type': TileType.MVT, 'tile_compression': Compression.GZIP,
        'min_lon_e7': 1156000000, 'min_lat_e7': -326000000,
        'max_lon_e7': 1163000000, 'max_lat_e7': -316000000,
        'center_zoom': 13, 'center_lon_e7': 1158571000, 'center_lat_e7': -319535000,
    }, {'attribution': '© OpenStreetMap contributors', 'name': 'fe2o3 sample'})
print(OUT, [ (t, len(tiles[t])) for t in sorted(tiles)])
