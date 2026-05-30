/*
Mindless NNUE trainer.

Architecture, quantization and save layout are pinned to match the engine's
`src/nnue.rs` exactly:
    (768 -> 128)x2 -> 1, dual-perspective, SCReLU
    QA = 255, QB = 64, SCALE = 400
    save order: l0w (QA), l0b (QA), l1w (QB), l1b (QA*QB)

This is bullet's stock `examples/simple.rs` with the data path and training
schedule made configurable via environment variables, so the same compiled
binary serves the brief smoke run and the full run without recompiling.
*/
use bullet_lib::{
    game::inputs::Chess768,
    nn::optimiser::AdamW,
    trainer::{
        save::SavedFormat,
        schedule::{TrainingSchedule, TrainingSteps, lr, wdl},
        settings::LocalSettings,
    },
    value::{ValueTrainerBuilder, loader},
};

const HIDDEN_SIZE: usize = 128;
const SCALE: i32 = 400;
const QA: i16 = 255;
const QB: i16 = 64;

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() {
    let data_path = std::env::var("MINDLESS_DATA").expect("set MINDLESS_DATA to the .data path");
    let net_id = std::env::var("MINDLESS_NET_ID").unwrap_or_else(|_| "mindless".to_string());
    let out_dir = std::env::var("MINDLESS_OUT_DIR").unwrap_or_else(|_| "checkpoints".to_string());
    let sbps: usize = env_or("MINDLESS_SBPS", 6104);
    let end_sb: usize = env_or("MINDLESS_END_SB", 40);
    let save_rate: usize = env_or("MINDLESS_SAVE_RATE", 10);
    let threads: usize = env_or("MINDLESS_THREADS", 4);
    let lr_start: f32 = env_or("MINDLESS_LR", 0.001);
    let lr_step: usize = env_or("MINDLESS_LR_STEP", 18);
    let wdl_val: f32 = env_or("MINDLESS_WDL", 0.75);

    println!(
        "mindless trainer: data={data_path} net_id={net_id} out={out_dir} \
         sbps={sbps} end_sb={end_sb} save_rate={save_rate} threads={threads} \
         lr={lr_start} lr_step={lr_step} wdl={wdl_val}"
    );

    let mut trainer = ValueTrainerBuilder::default()
        .dual_perspective()
        .optimiser(AdamW)
        .inputs(Chess768)
        .save_format(&[
            SavedFormat::id("l0w").round().quantise::<i16>(QA),
            SavedFormat::id("l0b").round().quantise::<i16>(QA),
            SavedFormat::id("l1w").round().quantise::<i16>(QB),
            SavedFormat::id("l1b").round().quantise::<i16>(QA * QB),
        ])
        .loss_fn(|output, target| output.sigmoid().squared_error(target))
        .build(|builder, stm_inputs, ntm_inputs| {
            let l0 = builder.new_affine("l0", 768, HIDDEN_SIZE);
            let l1 = builder.new_affine("l1", 2 * HIDDEN_SIZE, 1);

            let stm_hidden = l0.forward(stm_inputs).screlu();
            let ntm_hidden = l0.forward(ntm_inputs).screlu();
            let hidden_layer = stm_hidden.concat(ntm_hidden);
            l1.forward(hidden_layer)
        });

    let schedule = TrainingSchedule {
        net_id,
        eval_scale: SCALE as f32,
        steps: TrainingSteps {
            batch_size: 16_384,
            batches_per_superbatch: sbps,
            start_superbatch: 1,
            end_superbatch: end_sb,
        },
        wdl_scheduler: wdl::ConstantWDL { value: wdl_val },
        lr_scheduler: lr::StepLR { start: lr_start, gamma: 0.1, step: lr_step },
        save_rate,
    };

    let settings =
        LocalSettings { threads, test_set: None, output_directory: out_dir.as_str(), batch_queue_size: 64 };

    let data_loader = loader::DirectSequentialDataLoader::new(&[data_path.as_str()]);

    trainer.run(&schedule, &settings, &data_loader);
}
