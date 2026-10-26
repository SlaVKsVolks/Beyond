//! `horizon-client --benchmark SCENARIO`: the run `tools/perf_bench.py` drives from outside, driven
//! from inside, so anyone can measure their machine with nothing but the game. A thread sends the
//! same control commands `horizon-ctl` would, through the game's own command queue, and writes a
//! report (docs/benchmark-report.md) to `benchmarks/` in the game folder.
//!
//! The scenes, the world (seed 1234, creative) and every step and figure match `tools/perf_bench.py`,
//! so reports from the two can be compared.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use horizon_bench::{App, Frames, Kind, Report};
use serde_json::Value;

use crate::control::Request;

pub const SEED: i64 = 1234;
/// The world the benchmark creates under `saves/`, replaced on every run.
pub const WORLD: &str = "horizon-benchmark";
/// Written into the world so a later run only ever replaces its own world.
const MARKER: &str = "horizon-benchmark.txt";

/// A straight flight over terrain: name, start, end, pitch (`tools/graphics_bench.py`'s paths).
type Scene = (&'static str, [f64; 3], [f64; 3], f64);
pub const SCENES: [Scene; 4] = [
    (
        "village",
        [-960.0, 125.0, 80.0],
        [-960.0, 125.0, 272.0],
        35.0,
    ),
    (
        "forest",
        [-1100.0, 120.0, -560.0],
        [-1100.0, 120.0, -368.0],
        25.0,
    ),
    (
        "ocean",
        [-300.0, 85.0, -420.0],
        [-300.0, 85.0, -228.0],
        10.0,
    ),
    (
        "mountain",
        [560.0, 260.0, 860.0],
        [752.0, 260.0, 860.0],
        30.0,
    ),
];
const FLIGHT_SECONDS: f64 = 12.0;
const SETTLE: Duration = Duration::from_secs(2);
const LOAD_LIMIT: Duration = Duration::from_secs(600);

/// `standard` flies every scene; a scene's name flies only that one.
pub fn scenes(scenario: &str) -> Option<Vec<Scene>> {
    match scenario {
        "standard" => Some(SCENES.to_vec()),
        name => SCENES.iter().find(|s| s.0 == name).map(|s| vec![*s]),
    }
}

/// What the game knows about the run before it starts.
pub struct Setup {
    pub scenario: String,
    pub started: Instant,
    pub app: App,
    pub simulation_distance: u32,
    pub vsync: bool,
    /// `None`: unlimited.
    pub fps_cap: Option<u32>,
    pub offscreen: bool,
    pub game_dir: PathBuf,
}

/// Starts the driver; the game quits when it is done, and the report's path is printed.
pub fn start(requests: Sender<Request>, setup: Setup) {
    thread::Builder::new()
        .name("horizon-benchmark".into())
        .spawn(move || {
            let control = Control(requests);
            match run(&control, setup) {
                Ok(path) => println!("Horizon: benchmark report {}", path.display()),
                Err(e) => eprintln!("Horizon: benchmark failed: {e}"),
            }
            let _ = control.call("quit");
        })
        .expect("spawn the benchmark thread");
}

struct Control(Sender<Request>);

impl Control {
    fn call(&self, line: &str) -> Result<Value, String> {
        let (reply, response) = mpsc::channel();
        self.0
            .send(Request {
                line: line.to_owned(),
                reply,
            })
            .map_err(|_| "the game has exited".to_owned())?;
        // Flights and loads reply when they finish.
        let text = response
            .recv_timeout(LOAD_LIMIT)
            .map_err(|_| format!("{line}: no reply"))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        match value.get("error").and_then(Value::as_str) {
            Some(e) => Err(format!("{line}: {e}")),
            None => Ok(value),
        }
    }
    fn status(&self) -> Value {
        self.call("status").unwrap_or(Value::Null)
    }
}

fn run(control: &Control, setup: Setup) -> Result<PathBuf, String> {
    let scenes =
        scenes(&setup.scenario).ok_or_else(|| format!("unknown scenario {}", setup.scenario))?;
    let mut report = Report::new(Kind::Game, setup.app.clone());
    let pid = std::process::id();
    report.conditions.load_avg_1m = horizon_bench::load_average();
    report.conditions.power = horizon_bench::power_source().map(str::to_owned);
    let rss = RssSampler::start(pid);

    let title = poll(control, LOAD_LIMIT, |s| {
        s["screen"] == "title" && !s["renderer"].is_null()
    })?;
    report.metric("startup.title_ms", &[ms(setup.started.elapsed())]);
    let renderer = &title["renderer"];
    report.hardware.gpu = Some(horizon_bench::Gpu {
        name: text(&renderer["gpu"]),
        vendor: match text(&renderer["vendor"]) {
            // Metal reports no vendor ID.
            v if v.is_empty() && text(&renderer["gpu"]).starts_with("Apple") => "apple".into(),
            v => v,
        },
        driver: text(&renderer["driver"]),
        backend: text(&renderer["backend"]).to_lowercase(),
        kind: text(&renderer["device_type"]),
        vram_mb: None,
    });
    if let Some(gpu) = report.hardware.gpu.as_mut() {
        let vendor = horizon_telemetry::system::gpu_vendor(&gpu.name, 0);
        let vendor = if gpu.vendor.is_empty() {
            vendor
        } else {
            gpu.vendor.as_str()
        }
        .to_owned();
        if gpu.kind.contains("iscrete") {
            gpu.vram_mb = horizon_telemetry::system::vram_mb(&vendor);
        }
    }
    report.hardware.storage =
        horizon_telemetry::system::storage(&setup.game_dir).map(str::to_owned);
    report.hardware.display = serde_json::from_value(title["display"].clone()).ok();

    replace_world(&setup.game_dir)?;
    control.call(&format!("open {WORLD} {SEED} creative"))?;
    // The world opens on a worker: its folder may not exist yet when `open` replies.
    std::fs::create_dir_all(setup.game_dir.join("saves").join(WORLD)).map_err(|e| e.to_string())?;
    std::fs::write(
        setup.game_dir.join("saves").join(WORLD).join(MARKER),
        "Created by horizon-client --benchmark; replaced by the next run.\n",
    )
    .map_err(|e| e.to_string())?;
    let loaded = wait_loaded(control, |_| true)?;
    report.metric("startup.world_ms", &[ms(loaded - setup.started)]);

    let mut steady = Vec::new();
    for (name, start, end, pitch) in scenes {
        control.call("time 6000")?;
        let meshed = control.status()["meshed_sections"].as_f64().unwrap_or(0.0);
        let generated = control.status()["generated_chunks"].as_f64().unwrap_or(0.0);
        let [x, y, z] = start;
        control.call(&format!("tp {x} {y} {z}"))?;
        let teleported = Instant::now();
        // A zero-length flight holds the player at the start, flying.
        control.call(&format!("fly {x} {y} {z} {x} {y} {z} 0.1 {pitch}"))?;
        control.call(&format!("look {:.3} {pitch}", yaw(start, end)))?;
        let near = |p: &Value| {
            let p: Vec<f64> = p
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_f64)
                .collect();
            p.len() == 3 && (0..3).map(|i| (p[i] - start[i]).powi(2)).sum::<f64>() < 4.0
        };
        let loaded = wait_loaded(control, near)?;
        let load = loaded - teleported;
        report.metric(&format!("chunks.{name}.load_ms"), &[ms(load)]);
        let status = control.status();
        let meshes = status["meshed_sections"].as_f64().unwrap_or(0.0) - meshed;
        let chunks = status["generated_chunks"].as_f64().unwrap_or(0.0) - generated;
        report.metric(&format!("chunks.{name}.meshed_count"), &[meshes]);
        report.metric(&format!("chunks.{name}.generated_count"), &[chunks]);
        if load.as_secs_f64() > 0.0 {
            report.metric(
                &format!("chunks.{name}.generated_per_s"),
                &[chunks / load.as_secs_f64()],
            );
        }
        thread::sleep(SETTLE);
        let [ex, ey, ez] = end;
        let begin = Instant::now();
        let flight = control.call(&format!(
            "fly {x} {y} {z} {ex} {ey} {ez} {FLIGHT_SECONDS} {pitch} raw"
        ))?;
        steady.push((begin, Instant::now()));
        let series = |key: &str| -> Vec<f32> {
            flight[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_f64)
                .map(|v| v as f32)
                .collect()
        };
        let frames = Frames::of(
            name,
            0,
            &series("frame_ms"),
            &series("cpu_ms"),
            &series("gpu_ms"),
        )
        .ok_or("the flight drew no frames")?;
        report.metric(&format!("fps.{name}.avg_fps"), &[frames.avg_fps]);
        report.metric(&format!("fps.{name}.low_1pct_fps"), &[frames.low_1pct_fps]);
        report.metric(
            &format!("fps.{name}.low_01pct_fps"),
            &[frames.low_01pct_fps],
        );
        report.frames.push(frames);
    }

    let status = control.status();
    let renderer = &status["renderer"];
    // On Wayland the window is on a screen only once it has been shown: ask again.
    if report.hardware.display.is_none() {
        report.hardware.display = serde_json::from_value(status["display"].clone()).ok();
    }
    let c = &mut report.conditions;
    c.scenario = Some(setup.scenario.clone());
    c.seed = Some(SEED.to_string());
    c.render_distance = status["render_distance"].as_u64().map(|d| d as u32);
    c.simulation_distance = Some(setup.simulation_distance);
    c.window = serde_json::from_value(renderer["size"].clone()).ok();
    c.vsync = Some(setup.vsync);
    c.fps_cap = setup.fps_cap;
    c.offscreen = Some(setup.offscreen);
    c.repeats = Some(1);
    c.warmup_s = Some(SETTLE.as_secs_f64());
    if let Some(bytes) = renderer["gpu_allocated_bytes"].as_f64() {
        report.metric("memory.gpu_mb", &[bytes / f64::from(1 << 20)]);
    }
    let samples = rss.stop();
    if let Some(peak) = samples.iter().map(|s| s.1).max() {
        report.metric("memory.rss_peak_mb", &[peak as f64 / f64::from(1 << 20)]);
    }
    let mut during: Vec<u64> = samples
        .iter()
        .filter(|(t, _)| steady.iter().any(|(a, b)| a <= t && t <= b))
        .map(|s| s.1)
        .collect();
    during.sort_unstable();
    if let Some(&median) = during.get(during.len() / 2) {
        report.metric(
            "memory.rss_steady_mb",
            &[median as f64 / f64::from(1 << 20)],
        );
    }
    if let Ok(meta) = std::env::current_exe().and_then(std::fs::metadata) {
        report.metric("game.binary_bytes", &[meta.len() as f64]);
    }
    crate::telemetry::keep_benchmark(&crate::logging::game_home(), &report);
    report
        .write_to(&setup.game_dir.join("benchmarks"))
        .map_err(|e| e.to_string())
}

/// Removes the previous run's world, never one the benchmark didn't create.
fn replace_world(game_dir: &Path) -> Result<(), String> {
    let world = game_dir.join("saves").join(WORLD);
    if !world.exists() {
        return Ok(());
    }
    if !world.join(MARKER).exists() {
        return Err(format!(
            "{} exists and was not created by the benchmark: rename it",
            world.display()
        ));
    }
    std::fs::remove_dir_all(&world).map_err(|e| e.to_string())
}

fn poll(
    control: &Control,
    limit: Duration,
    ready: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        let status = control.status();
        if ready(&status) {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(format!("not ready after {} s", limit.as_secs()))
}

/// Fewer section meshes than this in a second is the game remeshing blocks that change (random
/// ticks), not loading.
const QUIET_MESHES_PER_S: f64 = 100.0;

/// Waits until the player is spawned where `near` says, nothing is queued, the resident and
/// generated chunk counts have not changed for a second and the last sections are drawn (they wait
/// for their neighbours' light, so after the counts settle); returns when the last of these
/// happened (`tools/perf_bench.py`'s `wait_loaded`). With `status.streaming`, the sections are
/// drawn once no stage has held work for half a second: blocks changing after the load (fluids
/// settling, random ticks) only remesh in bursts. Without it, meshing has to go quiet.
/// A quiet start after teleport is not a loaded scene: require enough resident terrain and
/// count progress, or the five-second fallback for a scene that was already full.
fn wait_loaded(control: &Control, near: impl Fn(&Value) -> bool) -> Result<Instant, String> {
    wait_loaded_with(
        || control.status(),
        near,
        LOAD_LIMIT,
        Instant::now,
        thread::sleep,
    )
}

fn wait_loaded_with(
    mut status: impl FnMut() -> Value,
    near: impl Fn(&Value) -> bool,
    limit: Duration,
    mut clock: impl FnMut() -> Instant,
    mut sleep: impl FnMut(Duration),
) -> Result<Instant, String> {
    let began = clock();
    let deadline = began + limit;
    let (mut last, mut changed, mut meshing) = (None, began, began);
    let mut previous: Option<(Instant, f64)> = None;
    let (mut first, mut moved) = (None, false);
    while clock() < deadline {
        let s = status();
        let now = clock();
        let position = &s["player"]["position"];
        let counts = (s["chunks"].as_u64(), s["generated_chunks"].as_u64());
        let min_chunks = s["render_distance"]
            .as_u64()
            .filter(|distance| (2..=64).contains(distance))
            .map(view_chunks)
            .unwrap_or(0);
        let meshed = s["meshed_sections"].as_f64();
        if position.is_null()
            || !near(position)
            || s["pending_chunks"] != 0
            || s["screen"].as_str().is_none()
            || s["screen"] == "title"
            || counts.0.is_none()
            || counts.1.is_none()
            || meshed.is_none()
            || min_chunks == 0
        {
            (last, changed, previous, meshing) = (None, now, None, now);
        } else {
            let resident = counts.0.unwrap_or(0);
            let first = *first.get_or_insert(counts);
            moved = moved
                || counts != first
                || (resident >= min_chunks && now - began > Duration::from_secs(5));
            if !moved || resident < min_chunks {
                (last, changed, previous, meshing) = (None, now, None, now);
                sleep(Duration::from_millis(50));
                continue;
            }
            if last != Some(counts) {
                (last, changed) = (Some(counts), now);
            }
            let meshed = meshed.unwrap_or(0.0);
            let quiet = if let Some(stages) = s["streaming"].as_object() {
                // Protos outside the view distance wait for neighbours that are never generated.
                if stages
                    .iter()
                    .any(|(k, v)| k != "protos" && v.as_u64().unwrap_or(0) > 0)
                {
                    meshing = now;
                }
                Duration::from_millis(500)
            } else {
                if let Some((at, before)) = previous
                    && meshed - before > QUIET_MESHES_PER_S * (now - at).as_secs_f64()
                {
                    meshing = now;
                }
                Duration::from_secs(1)
            };
            previous = Some((now, meshed));
            if now - changed >= Duration::from_secs(1) && now - meshing >= quiet {
                return Ok(changed.max(meshing));
            }
        }
        sleep(Duration::from_millis(50));
    }
    Err(format!("not loaded after {} s", limit.as_secs()))
}

/// 80% of the view circle, as in `tools/perf_bench.py`. The control protocol admits RD2..64.
fn view_chunks(distance: u64) -> u64 {
    (0.8 * std::f64::consts::PI * (distance as f64).powi(2)) as u64
}

/// Vanilla's yaw facing from `start` to `end`.
fn yaw(start: [f64; 3], end: [f64; 3]) -> f64 {
    (-(end[0] - start[0])).atan2(end[2] - start[2]).to_degrees()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// Samples the process's resident memory four times a second, as `tools/perf_bench.py` does.
struct RssSampler {
    stop: Sender<()>,
    thread: thread::JoinHandle<Vec<(Instant, u64)>>,
}

impl RssSampler {
    fn start(pid: u32) -> Self {
        let (stop, stopped) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut samples = Vec::new();
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(Duration::from_millis(250))
            {
                samples.extend(horizon_bench::rss_bytes(pid).map(|b| (Instant::now(), b)));
            }
            samples
        });
        Self { stop, thread }
    }
    fn stop(self) -> Vec<(Instant, u64)> {
        let _ = self.stop.send(());
        self.thread.join().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded_status(chunks: u64) -> Value {
        serde_json::json!({
            "player": {"position": [0, 0, 0]}, "screen": "playing",
            "pending_chunks": 0, "render_distance": 12,
            "chunks": chunks, "generated_chunks": chunks,
            "meshed_sections": 1000, "streaming": {"meshing": 0, "protos": 999}
        })
    }

    fn scripted_load(
        mut status: impl FnMut(Duration) -> Value,
        limit: Duration,
    ) -> (Result<Duration, String>, Duration) {
        let began = Instant::now();
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let result = wait_loaded_with(
            || status(elapsed.get()),
            |position| position[0] == 0,
            limit,
            || began + elapsed.get(),
            |duration| elapsed.set(elapsed.get() + duration),
        );
        (result.map(|at| at - began), elapsed.get())
    }

    #[test]
    fn empty_undersized_and_invalid_scenes_time_out() {
        let mut cases = vec![
            loaded_status(0),
            loaded_status(1),
            loaded_status(360),
            Value::Null,
        ];
        for field in [
            "chunks",
            "generated_chunks",
            "render_distance",
            "meshed_sections",
            "screen",
        ] {
            let mut status = loaded_status(637);
            status[field] = Value::Null;
            cases.push(status);
        }
        for distance in [0, 1, 65, u64::MAX] {
            let mut status = loaded_status(637);
            status["render_distance"] = distance.into();
            cases.push(status);
        }
        for status in cases {
            let (result, elapsed) = scripted_load(|_| status.clone(), Duration::from_secs(8));
            assert!(
                result.is_err(),
                "invalid or incomplete status was accepted: {status}"
            );
            assert_eq!(elapsed, Duration::from_secs(8));
        }
    }

    #[test]
    fn terrain_minimum_accepts_the_view_circle_boundary_after_progress() {
        for (distance, chunks, ready) in [
            (2, 9, false),
            (2, 10, true),
            (12, 360, false),
            (12, 361, true),
            (32, 2572, false),
            (32, 2573, true),
        ] {
            let (result, elapsed) = scripted_load(
                |at| {
                    let mut status = loaded_status(if at.is_zero() { 0 } else { chunks });
                    status["render_distance"] = distance.into();
                    status
                },
                Duration::from_secs(8),
            );
            if ready {
                assert_eq!(result.unwrap(), Duration::from_millis(50));
                assert_eq!(elapsed, Duration::from_millis(1050));
            } else {
                assert!(result.is_err());
                assert_eq!(elapsed, Duration::from_secs(8));
            }
        }
    }

    #[test]
    fn terrain_progress_waits_for_full_scene_and_stable_counts() {
        let (result, elapsed) = scripted_load(
            |at| {
                loaded_status(if at < Duration::from_millis(1500) {
                    0
                } else {
                    637
                })
            },
            Duration::from_secs(4),
        );
        assert_eq!(result.unwrap(), Duration::from_millis(1500));
        assert_eq!(elapsed, Duration::from_millis(2500));
    }

    #[test]
    fn already_full_scene_uses_five_second_fallback() {
        let (result, elapsed) = scripted_load(|_| loaded_status(637), Duration::from_secs(8));
        assert_eq!(result.unwrap(), Duration::from_millis(5050));
        assert_eq!(elapsed, Duration::from_millis(6050));
    }

    #[test]
    fn generated_progress_can_finish_an_already_resident_scene() {
        let (result, elapsed) = scripted_load(
            |at| {
                let mut status = loaded_status(637);
                status["generated_chunks"] = if at < Duration::from_millis(500) {
                    0
                } else {
                    1
                }
                .into();
                status
            },
            Duration::from_secs(4),
        );
        assert_eq!(result.unwrap(), Duration::from_millis(500));
        assert_eq!(elapsed, Duration::from_millis(1500));
    }

    #[test]
    fn pending_missing_player_wrong_position_and_title_reset_stability() {
        for blocked in 0..4 {
            let (result, elapsed) = scripted_load(
                |at| {
                    let mut status = loaded_status(if at.is_zero() { 0 } else { 637 });
                    if (Duration::from_secs(1)..Duration::from_millis(1500)).contains(&at) {
                        match blocked {
                            0 => status["pending_chunks"] = 1.into(),
                            1 => status["player"] = Value::Null,
                            2 => status["player"]["position"][0] = 100.into(),
                            _ => status["screen"] = "title".into(),
                        }
                    }
                    status
                },
                Duration::from_secs(4),
            );
            assert_eq!(result.unwrap(), Duration::from_millis(1500));
            assert_eq!(elapsed, Duration::from_millis(2500));
        }
    }

    #[test]
    fn active_stages_wait_for_half_second_quiet_but_protos_do_not() {
        let (result, elapsed) = scripted_load(
            |at| {
                let mut status = loaded_status(if at.is_zero() { 0 } else { 637 });
                if at < Duration::from_millis(1500) {
                    status["streaming"]["meshing"] = 1.into();
                }
                status
            },
            Duration::from_secs(4),
        );
        assert_eq!(result.unwrap(), Duration::from_millis(1450));
        assert_eq!(elapsed, Duration::from_millis(1950));
    }

    #[test]
    fn legacy_status_waits_for_a_second_of_mesh_quiet() {
        let (result, elapsed) = scripted_load(
            |at| {
                let mut status = loaded_status(if at.is_zero() { 0 } else { 637 });
                status.as_object_mut().unwrap().remove("streaming");
                status["meshed_sections"] = (at.as_millis().min(1500) as u64 * 10).into();
                status
            },
            Duration::from_secs(4),
        );
        assert_eq!(result.unwrap(), Duration::from_millis(1500));
        assert_eq!(elapsed, Duration::from_millis(2500));
    }

    #[test]
    fn scene_does_not_load_during_the_quiet_start_after_teleport() {
        // Exercise the production Control/response path without starting a game.
        let began = Instant::now();
        let (requests, received) = mpsc::channel::<Request>();
        thread::scope(|scope| {
            scope.spawn(move || {
                for request in received {
                    let chunks = if began.elapsed() < Duration::from_millis(1500) {
                        0
                    } else {
                        637
                    };
                    let status = serde_json::json!({
                        "player": {"position": [0, 0, 0]}, "screen": "playing",
                        "pending_chunks": 0, "render_distance": 12,
                        "chunks": chunks, "generated_chunks": chunks,
                        "meshed_sections": 1000, "streaming": {"meshing": 0}
                    });
                    let _ = request.reply.send(status.to_string());
                }
            });
            let loaded = wait_loaded(&Control(requests), |_| true).unwrap();
            assert!(
                loaded.duration_since(began) >= Duration::from_millis(1500),
                "the initial empty scene was treated as loaded"
            );
        });
    }

    #[test]
    fn scenarios() {
        assert_eq!(scenes("standard").unwrap().len(), 4);
        assert_eq!(scenes("ocean").unwrap()[0].0, "ocean");
        assert!(scenes("nether").is_none());
    }

    #[test]
    fn yaw_matches_vanilla() {
        // South (+Z) is 0, west (-X) 90, east (+X) -90.
        assert!((yaw([0.0; 3], [0.0, 0.0, 10.0])).abs() < 1e-9);
        assert!((yaw([0.0; 3], [-10.0, 0.0, 0.0]) - 90.0).abs() < 1e-9);
        assert!((yaw([0.0; 3], [10.0, 0.0, 0.0]) + 90.0).abs() < 1e-9);
    }

    #[test]
    fn only_replaces_its_own_world() {
        let dir =
            std::env::temp_dir().join(format!("horizon-benchmark-test-{}", std::process::id()));
        let world = dir.join("saves").join(WORLD);
        std::fs::create_dir_all(&world).unwrap();
        assert!(
            replace_world(&dir).is_err(),
            "a player's world of that name stays"
        );
        assert!(world.exists());
        std::fs::write(world.join(MARKER), "").unwrap();
        replace_world(&dir).unwrap();
        assert!(!world.exists());
        replace_world(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
