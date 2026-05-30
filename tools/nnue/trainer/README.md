# NNUE trainer (provenance & reproduction)

The Milestone-6 network `tools/nnue/nets/mindless-v1.nnue` was trained with
[**bullet**](https://github.com/jw1912/bullet) (a fast, standard NNUE trainer)
on a GPU. bullet itself is large and is **not** vendored into this repo; only the
two small source files needed to reproduce the net live here:

| File         | Role                                                                 |
|--------------|----------------------------------------------------------------------|
| `mindless.rs`| bullet training example — the exact architecture, quantization, save order, and schedule used. |
| `refeval.rs` | Independent reference inference used for the mandatory **eval-match** check (see below). |

Both are bullet *examples* (they depend on `bullet_lib`); drop them into a bullet
checkout to build. The net architecture and quantization are pinned to match the
engine's `src/nnue.rs` exactly — change one and you must change the other.

## The contract (must match `src/nnue.rs`)

- Architecture: `(768 -> 128)x2 -> 1`, dual-perspective, **SCReLU** activation.
- Inputs: bullet `Chess768` (plain piece-square, `64*piece + square`, no buckets).
- Quantization: `QA = 255` (feature transformer), `QB = 64` (output), eval `SCALE = 400`.
- `quantised.bin` save order: `l0w` (×QA), `l0b` (×QA), `l1w` (×QB), `l1b` (×QA·QB).
- Net file size: 197,440 bytes (the 197,378-byte network padded up to a multiple of 64).

## Reproduction

1. **CUDA toolkit** 12.6 at `/usr/local/cuda` (NVIDIA official package). The GPU
   driver alone is not enough — bullet links `cuda`/`cudart`/`nvrtc`/`cublas`.
2. **bullet** — clone and pin the commit this net was trained with:
   ```sh
   git clone https://github.com/jw1912/bullet ~/bullet
   git -C ~/bullet checkout d372d48          # the exact commit used
   ```
   Copy these two files into `~/bullet/examples/` and register them in
   `~/bullet/crates/bullet_lib/Cargo.toml` as `[[example]]` entries
   (`name = "mindless"` / `"refeval"`, `path = "../../examples/<file>"`).
3. **Generate data** with the engine's self-play (already used for v1 —
   42,125,153 positions, settled/quiet/non-mate, white-relative `FEN | cp | wdl`):
   ```sh
   mindless datagen --games <N> --nodes 5000 --out mindless-v1.txt
   ```
4. **Convert & shuffle** to bullet's binary format (no GPU needed):
   ```sh
   bullet-utils convert --from text --input mindless-v1.txt --output mindless-v1.data
   bullet-utils shuffle --input mindless-v1.data --output mindless-v1.shuf.data --mem-used-mb 4096
   ```
5. **Train** (GPU). The example reads its schedule from environment variables;
   v1 used 40 epoch-sized superbatches, AdamW, StepLR, WDL 0.75:
   ```sh
   cd ~/bullet
   CUDA_PATH=/usr/local/cuda LD_LIBRARY_PATH=/usr/local/cuda/lib64:$LD_LIBRARY_PATH \
   MINDLESS_DATA=$PWD/data/mindless-v1.shuf.data MINDLESS_NET_ID=mindless-v1 \
   MINDLESS_SBPS=2560 MINDLESS_END_SB=40 MINDLESS_SAVE_RATE=10 MINDLESS_THREADS=8 \
   MINDLESS_LR=0.001 MINDLESS_LR_STEP=18 MINDLESS_WDL=0.75 \
   cargo run -r --features cuda --example mindless
   ```
   The quantised net is written to `checkpoints/mindless-v1-40/quantised.bin`.
6. **Eval-match (mandatory).** The engine must reproduce bullet's reference
   inference to the centipawn before any net is trusted:
   ```sh
   cargo run -r --features cuda --example refeval -- net.bin < fens.txt   # reference
   mindless eval net.bin < fens.txt                                       # engine
   # the two centipawn columns must be identical
   ```
   For v1 this matched **exactly** on 18 varied positions (both side-to-move
   colours, openings through endgames).

Copy the resulting `quantised.bin` to `tools/nnue/nets/<name>.nnue`. The engine
embeds that file at build time (`src/nnue.rs`), so rebuilding ships the new net.
