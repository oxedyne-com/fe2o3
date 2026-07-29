# oxedyne_fe2o3_infer

Convolutional inference on the CPU, in safe Rust, with no dependency beyond
`oxedyne_fe2o3_core`.

The crate carries the numerics for two small networks -- a face detector and a
face embedder -- and the pieces around them: an ONNX subset loader, the
anchor-free decode and suppression a detector needs, the similarity transform
and warp an embedder's preprocessing needs, and a hundred and twenty-eight
dimensional unit vector at the end that two faces can be compared with.

It owns nothing. Weights arrive as `&[u8]`, images arrive as pixels, and where
they came from and how the work is spread across cores are the caller's
business.

## Using it

```rust
use oxedyne_fe2o3_infer::prelude::*;

let cpu = Cpu::detect();
let det = res!(Detector::load(&detector_onnx));
let emb = res!(Embedder::load(&embedder_onnx));

// Fit the photograph into the detector's canvas.
let img = res!(Image::new(&rgb, width, height, 3));
let (canvas, lb) = res!(letterbox(&img, 640, 640));
let view = res!(Image::new(&canvas, 640, 640, 3));

// Detect, then embed each face out of the original photograph.
for d in res!(det.detect(cpu, &view, &DetectorOptions::default())) {
    let d = d.unletterbox(&lb);
    let e = res!(emb.embed(cpu, &img, &d.landmarks));
    // `cosine(&e, &other)` compares two faces.
}
```

## The two networks

Neither is in this repository. Weights are not source, and the pair comes to
thirty-nine megabytes.

| | Detector | Embedder |
|---|---|---|
| File | `face_detection_yunet_2023mar.onnx` | `face_recognition_sface_2021dec.onnx` |
| Bytes | 232,589 | 38,696,353 |
| SHA-256 | `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4` | `0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79` |
| Licence | MIT | Apache-2.0 |
| Parameters | 53,121 | 9,671,000 |
| Output | boxes, scores, five landmarks | 128 dimensions |

Both come from the OpenCV model zoo, over git-lfs media URLs:

```
https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx
https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_recognition_sface/face_recognition_sface_2021dec.onnx
```

The plain `raw.githubusercontent.com` URL answers a git-lfs pointer, not the
model. Each hash above is also the `oid` in that pointer, so fetching both and
comparing is a provenance check as well as an integrity one.

Redistributing either is permitted, with the licence text and notices carried
along -- Apache-2.0 §4 for the embedder, the copyright notice for the detector.
Note separately that both were trained on corpora whose own terms are
research-only; the distributor's grant is what a licence question turns on, but
the provenance is worth knowing.

**Channel order differs between them and neither says so.** The detector was
exported against blue-green-red input and the embedder against red-green-blue.
Both entry points here take ordinary red-green-blue pixels and reorder for the
network, so a caller never has to know, but anyone feeding the graph directly
does.

## Testing

```bash
CARGO_TARGET_DIR=~/.cache/cargo-targets/fe2o3_infer_target \
  FE2O3_INFER_MODELS=/path/to/the/onnx/files \
  cargo test --release -p oxedyne_fe2o3_infer
```

Without `FE2O3_INFER_MODELS` the tests that want a model report that they were
skipped and pass. `tests/models.rs` carries a hundred and twenty-eight floats
and seventy-two summary values recorded from an independent implementation
(tract 0.23.4) on a fixed input, so the check is against something other than
this crate's own answer.

`tests/guard.rs` is a performance guard rather than a correctness one, and it
is not optional. The register tile in the matrix kernel sits on a cliff:
whether the accumulator lives in vector registers or spills to the stack is an
all-or-nothing decision the code generator makes, adjacent tile heights differ
by a factor of thirty-two, and a compiler upgrade can move the boundary without
changing a line. The second guard catches `mul_add` reaching the path that has
no fused multiply-add, where it becomes a library call and costs the same
thirty times.

## Measured

AMD Ryzen 7 6800H, one core, stock `x86-64` target with no `-C target-cpu`.

| | this crate | tract 0.23.4 |
|---|---|---|
| Detector, 640×640 | 37.4 ms | 99.4 ms |
| Embedder, 112×112 | 16.8 ms | 46.7 ms |
| Matrix kernel, weighted over the embedder's layers | 39.4 GMAC/s | 12.3 GMAC/s |
| The same on the baseline path | 11.0 GMAC/s | -- |

Agreement with tract over seventy-eight photographs: largest absolute
difference in any detector head, 1.9 × 10⁻⁵; every decoded box identical to
five decimal places of intersection over union. Over ninety-three faces: cosine
similarity between the two embeddings 1.000000000, largest difference in any
component of the unit vector 2.1 × 10⁻⁶.

## Safety

The crate is `#![deny(unsafe_code)]` with exactly one documented exception, in
`kern::run`, where calling a `#[target_feature]` function from an unfeatured
context requires the token even though the body is entirely safe. There are no
raw pointers, no intrinsics and no hand-written assembly. There is no
`unwrap()`, no `expect()`, and no `?`.
