use std::{env, time::Duration};

use criterion::Criterion;

#[path = "opb/consumer.rs"]
mod consumer;
#[path = "opb/input.rs"]
mod input;
#[path = "opb/workload.rs"]
mod workload;

use input::{Job, Prepared};

fn roundtrip<C: consumer::Consumer>(job: &Job, out: &mut C) {
    if job.workload == "open-drop" {
        workload::open(job, out);
    } else {
        let mut p = Prepared::new(job);
        workload::operation(job, &mut p, out);
        drop(p);
    }
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("ready") {
        // Executable/loader readiness only. No waveform API here.
        println!("OPB ready");
        return;
    }
    assert!(args.len() == 4, "profile measure|verify run.json job-index");
    let job = Job::read(&args[2], args[3].parse().unwrap());
    match args[1].as_str() {
        "measure" => measure(&job),
        "verify" => {
            let mut out = consumer::Verify::new(std::io::stdout().lock());
            roundtrip(&job, &mut out);
            out.finish();
        }
        _ => panic!("unknown mode"),
    }
}

fn measure(job: &Job) {
    let settings = &job.settings;
    let mut c = Criterion::default()
        .without_plots()
        .output_directory(&job.raw)
        .sample_size(settings["sample_size"].as_u64().unwrap() as usize)
        .nresamples(settings["nresamples"].as_u64().unwrap() as usize)
        .warm_up_time(Duration::from_secs_f64(
            settings["warm_up_seconds"].as_f64().unwrap(),
        ))
        .measurement_time(Duration::from_secs_f64(
            settings["measurement_seconds"].as_f64().unwrap(),
        ));
    let mut group = c.benchmark_group(&job.backend);
    match job.boundary.as_str() {
        "R" => {
            group.bench_function(&job.id, |b| {
                b.iter(|| roundtrip(job, &mut consumer::Timing))
            });
        }
        "P" | "S" => {
            group.bench_function(&job.id, |b| {
                b.iter_custom(|iters| {
                    let mut duration = Duration::ZERO;
                    for _ in 0..iters {
                        let mut p = Prepared::new(job);
                        duration += workload::operation(job, &mut p, &mut consumer::Timing);
                        drop(p);
                    }
                    duration
                })
            });
        }
        "T" => {
            let mut p = Prepared::new(job);
            group.bench_function(&job.id, |b| {
                b.iter(|| workload::execute(job, &mut p, &mut consumer::Timing))
            });
        }
        _ => panic!("unknown boundary"),
    }
    group.finish();
    c.final_summary();
}
