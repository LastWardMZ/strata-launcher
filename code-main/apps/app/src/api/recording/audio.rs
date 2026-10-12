//! Audio de la grabación (FFmpeg no tiene captura de audio de Windows): WASAPI captura el audio del proceso de
//! Minecraft (y, opcionalmente, el micrófono) y un escritor lo envía a FFmpeg por una tubería con nombre a ritmo
//! de tiempo real. Si el juego no suena, se envía silencio para que el audio nunca se desincronice del vídeo.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

pub type Queue = Arc<Mutex<VecDeque<u8>>>;

pub const RATE: usize = 48_000;
/// PCM `f32` estéreo.
pub const FRAME_BYTES: usize = 8;
/// Si se acumula más de este audio sin enviar, se descarta lo más antiguo (evita retrasos crecientes).
const MAX_QUEUED_BYTES: usize = RATE * FRAME_BYTES;

pub fn new_queue() -> Queue {
    Arc::new(Mutex::new(VecDeque::new()))
}

fn wave_format() -> WaveFormat {
    WaveFormat::new(32, 32, &SampleType::Float, RATE, 2, None)
}

#[cfg(test)]
static DEBUG_LAST_SECOND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64 % 100_000
}

/// Solo para diagnóstico en pruebas: bytes capturados, pico capturado y bytes no nulos enviados.
#[cfg(test)]
pub(super) static DEBUG: [std::sync::atomic::AtomicU64; 4] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];

fn push(queue: &Queue, local: &mut VecDeque<u8>) {
    #[cfg(test)]
    {
        let peak = local.iter().copied().collect::<Vec<u8>>().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs()).fold(0f32, f32::max);
        DEBUG[0].fetch_add(local.len() as u64, Ordering::Relaxed);
        DEBUG[1].fetch_max((peak * 1000.0) as u64, Ordering::Relaxed);
    }
    let mut queue = queue.lock().unwrap();
    queue.extend(local.drain(..));
    if queue.len() > MAX_QUEUED_BYTES {
        let excess = queue.len() - MAX_QUEUED_BYTES / 2;
        queue.drain(..excess - excess % FRAME_BYTES);
    }
}

fn capture_loop(client: &AudioClient, queue: &Queue, stop: &AtomicBool) -> Result<(), String> {
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    client.start_stream().map_err(|e| e.to_string())?;
    let mut local: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        // Se vacían todos los paquetes pendientes antes de esperar al siguiente aviso.
        while capture.get_next_packet_size().map_err(|e| e.to_string())?.unwrap_or(0) > 0 {
            capture.read_from_device_to_deque(&mut local).map_err(|e| e.to_string())?;
        }
        if !local.is_empty() {
            push(queue, &mut local);
        }
        // El aviso no llega mientras no suena nada: el tiempo de espera agotado es normal.
        let _ = event.wait_for_event(100);
    }
    let _ = client.stop_stream();
    Ok(())
}

/// Hilo de captura. Devuelve error si WASAPI no se puede inicializar (se espera hasta 5 s a que arranque).
fn spawn_capture(
    name: &str,
    make_client: impl FnOnce() -> Result<AudioClient, String> + Send + 'static,
    queue: Queue,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>, String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let thread_name = name.to_string();
    let handle = std::thread::Builder::new()
        .name(thread_name.clone())
        .spawn(move || {
            let init = (|| -> Result<AudioClient, String> {
                initialize_mta().ok().map_err(|e| e.to_string())?;
                let mut client = make_client()?;
                let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
                client.initialize_client(&wave_format(), &Direction::Capture, &mode).map_err(|e| e.to_string())?;
                Ok(client)
            })();
            match init {
                Ok(client) => {
                    let _ = ready_tx.send(Ok(()));
                    if let Err(error) = capture_loop(&client, &queue, &stop) {
                        tracing::warn!("Captura de audio ({thread_name}) detenida: {error}");
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("La captura de audio no arrancó a tiempo".into()),
    }
}

/// Audio del proceso del juego (y de sus procesos hijos). Requiere Windows 10 versión 2004 o posterior.
pub fn start_game_capture(pid: u32, queue: Queue, stop: Arc<AtomicBool>) -> Result<std::thread::JoinHandle<()>, String> {
    spawn_capture(
        "strata-audio-juego",
        move || AudioClient::new_application_loopback_client(pid, true).map_err(|e| e.to_string()),
        queue,
        stop,
    )
}

/// Micrófono predeterminado del sistema.
pub fn start_microphone_capture(queue: Queue, stop: Arc<AtomicBool>) -> Result<std::thread::JoinHandle<()>, String> {
    spawn_capture(
        "strata-audio-micro",
        || {
            let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
            let device = enumerator.get_default_device(&Direction::Capture).map_err(|_| "No hay micrófono predeterminado".to_string())?;
            device.get_iaudioclient().map_err(|e| e.to_string())
        },
        queue,
        stop,
    )
}

/// Crea el servidor de la tubería. Debe existir antes de lanzar FFmpeg, que se conecta como cliente.
pub fn create_pipe(name: &str) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new().access_inbound(false).access_outbound(true).reject_remote_clients(true).first_pipe_instance(true).create(name)
}

/// Cuántos fotogramas hay que haber escrito `elapsed` después de empezar.
pub fn frames_due(elapsed: Duration) -> u64 {
    (elapsed.as_secs_f64() * RATE as f64) as u64
}

/// Si el audio escrito va más de esto por detrás del reloj, la captura se ha detenido y se rellena con silencio.
pub const STALL_FRAMES: u64 = RATE as u64 * 15 / 100;

/// Saca de la cola todo lo que haya (solo fotogramas completos).
pub fn take_available(queue: &Queue) -> Vec<u8> {
    let mut queue = queue.lock().unwrap();
    let whole = queue.len() - queue.len() % FRAME_BYTES;
    queue.drain(..whole).collect()
}

/// Silencio que falta para alcanzar el reloj, o 0 si no hay un corte apreciable de la captura.
pub fn silence_needed(written: u64, due: u64) -> u64 {
    if due > written + STALL_FRAMES { due - written } else { 0 }
}

/// Envía a FFmpeg el audio capturado **en cuanto llega** (sin colchón, para no retrasarlo respecto a la imagen) y solo
/// rellena con silencio si la captura se detiene, de modo que el audio nunca se quede por detrás del tiempo real.
/// Cerrar la tubería avisa a FFmpeg de que no hay más audio.
pub async fn write_pcm(mut pipe: NamedPipeServer, queue: Queue, stop: Arc<AtomicBool>) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(20), pipe.connect())
        .await
        .map_err(|_| "FFmpeg no se conectó a la tubería de audio".to_string())?
        .map_err(|e| e.to_string())?;
    // Lo capturado antes de que FFmpeg esté listo ya es pasado: se descarta para no desplazar el audio.
    queue.lock().unwrap().clear();
    let started = Instant::now();
    #[cfg(test)]
    println!("TUBERÍA CONECTADA a las {} ms", now_ms());
    let mut written: u64 = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !stop.load(Ordering::Relaxed) {
        tick.tick().await;
        let mut chunk = take_available(&queue);
        #[cfg(test)]
        {
            DEBUG[2].fetch_add(chunk.iter().filter(|b| **b != 0).count() as u64, Ordering::Relaxed);
            if chunk.chunks_exact(4).any(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs() > 0.05) {
                let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                if DEBUG_LAST_SECOND.swap(ms / 1000, Ordering::Relaxed) != ms / 1000 {
                    println!("LLEGADA AL ESCRITOR: {} ms tras el inicio del segundo", ms % 1000);
                }
            }
        }
        written += (chunk.len() / FRAME_BYTES) as u64;
        let silence = silence_needed(written, frames_due(started.elapsed()));
        if silence > 0 {
            chunk.resize(chunk.len() + silence as usize * FRAME_BYTES, 0);
            written += silence;
        }
        if chunk.is_empty() {
            continue;
        }
        // Si FFmpeg ya terminó, la tubería se cierra y el error es esperable.
        if let Err(error) = pipe.write_all(&chunk).await {
            #[cfg(test)]
            println!("ESCRITURA EN LA TUBERÍA FALLÓ a las {} ms tras {written} fotogramas ({:.2} s de audio): {error}", now_ms(), written as f64 / RATE as f64);
            let _ = error;
            return Ok(());
        }
    }
    let _ = pipe.shutdown().await;
    Ok(())
}

/// Solo para pruebas: reproduce por el dispositivo predeterminado un pitido de 1 kHz durante los primeros 200 ms de
/// cada segundo UTC y silencio el resto, hasta que se pida parar.
#[cfg(test)]
pub(super) fn play_second_beeps_for_tests(stop: Arc<AtomicBool>) {
    initialize_mta().ok().unwrap();
    let enumerator = DeviceEnumerator::new().unwrap();
    let device = enumerator.get_default_device(&Direction::Render).unwrap();
    let mut client = device.get_iaudioclient().unwrap();
    let format = wave_format();
    let block = format.get_blockalign() as usize;
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(&format, &Direction::Render, &mode).unwrap();
    let event = client.set_get_eventhandle().unwrap();
    let render = client.get_audiorenderclient().unwrap();
    let mut phase = 0f64;
    let mut fill = |frames: usize| {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let mut data = vec![0u8; frames * block];
        for (index, frame) in data.chunks_exact_mut(block).enumerate() {
            // Cada fotograma del búfer corresponde a un instante: `now_ms` + su posición.
            let ms = (now_ms + (index as u64 * 1000) / RATE as u64) % 1000;
            let amplitude = if ms < 200 { 0.3 } else { 0.0 };
            let sample = ((phase * 2.0 * std::f64::consts::PI * 1000.0).sin() * amplitude) as f32;
            phase = (phase + 1.0 / RATE as f64) % 1.0;
            for channel in frame.chunks_exact_mut(4) {
                channel.copy_from_slice(&sample.to_le_bytes());
            }
        }
        render.write_to_device(frames, &data, None).unwrap();
    };
    // Búfer pequeño: así lo que suena corresponde a «ahora» con poco margen.
    let space = client.get_available_space_in_frames().unwrap();
    fill((space as usize).min(RATE / 100));
    client.start_stream().unwrap();
    while !stop.load(Ordering::Relaxed) {
        if event.wait_for_event(1000).is_err() {
            break;
        }
        let space = client.get_available_space_in_frames().unwrap();
        fill((space as usize).min(RATE / 100));
    }
    let _ = client.stop_stream();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prueba manual (ignorada): captura 4 s del audio del proceso `STRATA_TEST_PID` y muestra cuánto llegó.
    #[test]
    #[ignore = "manual: necesita un proceso que esté sonando"]
    fn process_audio_capture_receives_samples() {
        let pid: u32 = std::env::var("STRATA_TEST_PID").expect("STRATA_TEST_PID").parse().unwrap();
        let queue = new_queue();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = start_game_capture(pid, queue.clone(), stop.clone()).expect("iniciar la captura");
        let mut total = 0usize;
        let mut peak = 0f32;
        let mut nonzero_chunks = 0;
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(100));
            let chunk: Vec<u8> = queue.lock().unwrap().drain(..).collect();
            total += chunk.len();
            let chunk_peak = chunk.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs()).fold(0f32, f32::max);
            if chunk_peak > 0.001 {
                nonzero_chunks += 1;
            }
            peak = peak.max(chunk_peak);
        }
        stop.store(true, Ordering::Relaxed);
        let _ = thread.join();
        println!("AUDIO: bytes={total} (≈{:.2} s) pico={peak:.4} bloques_con_sonido={nonzero_chunks}", total as f64 / (RATE * FRAME_BYTES) as f64);
        assert!(total > 0, "llegan muestras");
        assert!(peak > 0.01, "hay sonido (pico {peak})");
    }

    /// Prueba manual (ignorada): ¿se oye algo en el dispositivo de reproducción predeterminado? Sirve para saber si un
    /// silencio en la captura por proceso es culpa del proceso (no suena) o de la captura.
    #[test]
    #[ignore = "manual: necesita que algo esté sonando"]
    fn system_loopback_hears_sound() {
        initialize_mta().ok().unwrap();
        let enumerator = DeviceEnumerator::new().unwrap();
        let device = enumerator.get_default_device(&Direction::Render).unwrap();
        println!("DISPOSITIVO: {:?}", device.get_friendlyname());
        let mut client = device.get_iaudioclient().unwrap();
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
        client.initialize_client(&wave_format(), &Direction::Capture, &mode).unwrap();
        let event = client.set_get_eventhandle().unwrap();
        let capture = client.get_audiocaptureclient().unwrap();
        client.start_stream().unwrap();
        let mut peak = 0f32;
        let mut bytes = 0usize;
        let started = Instant::now();
        let mut local: VecDeque<u8> = VecDeque::new();
        while started.elapsed() < Duration::from_secs(4) {
            while capture.get_next_packet_size().unwrap().unwrap_or(0) > 0 {
                capture.read_from_device_to_deque(&mut local).unwrap();
            }
            bytes += local.len();
            let data: Vec<u8> = local.drain(..).collect();
            peak = data.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs()).fold(peak, f32::max);
            let _ = event.wait_for_event(100);
        }
        client.stop_stream().unwrap();
        println!("SISTEMA: bytes={bytes} pico={peak:.4}");
        assert!(peak > 0.01, "se oye algo en el dispositivo predeterminado (pico {peak})");
    }

    /// Prueba manual (ignorada) y autosuficiente: este proceso reproduce un tono de 1 kHz por el dispositivo
    /// predeterminado y la captura por proceso (la que usa la grabación) debe oírlo.
    #[test]
    #[ignore = "manual: reproduce un tono en el dispositivo de audio"]
    fn process_loopback_captures_a_tone_played_by_this_process() {
        let stop_play = Arc::new(AtomicBool::new(false));
        let playing = stop_play.clone();
        let player = std::thread::spawn(move || {
            initialize_mta().ok().unwrap();
            let enumerator = DeviceEnumerator::new().unwrap();
            let device = enumerator.get_default_device(&Direction::Render).unwrap();
            let mut client = device.get_iaudioclient().unwrap();
            let format = wave_format();
            let block = format.get_blockalign() as usize;
            let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
            client.initialize_client(&format, &Direction::Render, &mode).unwrap();
            let event = client.set_get_eventhandle().unwrap();
            let render = client.get_audiorenderclient().unwrap();
            let mut phase = 0f64;
            let mut fill = |frames: usize| {
                let mut data = vec![0u8; frames * block];
                for frame in data.chunks_exact_mut(block) {
                    let sample = ((phase * 2.0 * std::f64::consts::PI * 1000.0).sin() * 0.3) as f32;
                    phase = (phase + 1.0 / RATE as f64) % 1.0;
                    for channel in frame.chunks_exact_mut(4) {
                        channel.copy_from_slice(&sample.to_le_bytes());
                    }
                }
                render.write_to_device(frames, &data, None).unwrap();
            };
            let space = client.get_available_space_in_frames().unwrap();
            fill(space as usize);
            client.start_stream().unwrap();
            while !playing.load(Ordering::Relaxed) {
                if event.wait_for_event(1000).is_err() {
                    break;
                }
                let space = client.get_available_space_in_frames().unwrap();
                fill(space as usize);
            }
            let _ = client.stop_stream();
        });
        std::thread::sleep(Duration::from_millis(600));

        let queue = new_queue();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = start_game_capture(std::process::id(), queue.clone(), stop.clone()).expect("iniciar la captura");
        let mut peak = 0f32;
        let mut bytes = 0usize;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            let chunk: Vec<u8> = queue.lock().unwrap().drain(..).collect();
            bytes += chunk.len();
            peak = chunk.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs()).fold(peak, f32::max);
        }
        stop.store(true, Ordering::Relaxed);
        stop_play.store(true, Ordering::Relaxed);
        let _ = thread.join();
        let _ = player.join();
        println!("TONO: bytes={bytes} pico={peak:.3} (el tono suena a 0,3)");
        assert!(peak > 0.2 && peak < 0.45, "la captura por proceso oye el tono reproducido (pico {peak})");
    }

    /// Prueba manual (ignorada): ¿cuánto tarda en llegar a mi código un pitido que empieza justo al comenzar el segundo UTC?
    #[test]
    #[ignore = "manual: reproduce pitidos en el dispositivo de audio"]
    fn process_loopback_latency_for_second_beeps() {
        let stop_tone = Arc::new(AtomicBool::new(false));
        let tone_stop = stop_tone.clone();
        let tone = std::thread::spawn(move || play_second_beeps_for_tests(tone_stop));
        std::thread::sleep(Duration::from_millis(700));
        let queue = new_queue();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = start_game_capture(std::process::id(), queue.clone(), stop.clone()).expect("iniciar la captura");
        let mut arrivals: Vec<u64> = Vec::new();
        let mut last_second = u64::MAX;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(2));
            let chunk: Vec<u8> = queue.lock().unwrap().drain(..).collect();
            let loud = chunk.chunks_exact(4).any(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs() > 0.05);
            if loud {
                let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                if ms / 1000 != last_second {
                    last_second = ms / 1000;
                    arrivals.push(ms % 1000);
                }
            }
        }
        stop.store(true, Ordering::Relaxed);
        stop_tone.store(true, Ordering::Relaxed);
        let _ = thread.join();
        let _ = tone.join();
        println!("LATENCIA de la captura (ms tras el inicio del pitido): {arrivals:?}");
        assert!(!arrivals.is_empty());
    }

    #[test]
    fn pacing_follows_wall_clock_time() {
        assert_eq!(frames_due(Duration::from_secs(1)), 48_000);
        assert_eq!(frames_due(Duration::from_millis(20)), 960);
        assert_eq!(frames_due(Duration::ZERO), 0);
    }

    #[test]
    fn captured_audio_is_sent_immediately_and_in_order() {
        let queue = new_queue();
        assert!(take_available(&queue).is_empty(), "sin audio no se inventa nada");
        queue.lock().unwrap().extend((1..=16u8).collect::<Vec<_>>()); // 2 fotogramas
        assert_eq!(take_available(&queue), (1..=16u8).collect::<Vec<_>>());
        assert!(queue.lock().unwrap().is_empty());
        queue.lock().unwrap().extend(vec![7u8; 10 * FRAME_BYTES]);
        assert_eq!(take_available(&queue).len(), 10 * FRAME_BYTES, "se entrega todo lo disponible, sin colchón");
    }

    #[test]
    fn a_partial_frame_is_never_split() {
        let queue = new_queue();
        queue.lock().unwrap().extend(vec![9u8; FRAME_BYTES + 3]);
        let out = take_available(&queue);
        assert_eq!(out, vec![9u8; FRAME_BYTES]);
        assert_eq!(queue.lock().unwrap().len(), 3, "los bytes sueltos esperan al resto del fotograma");
    }

    #[test]
    fn silence_is_only_added_when_the_capture_really_stalls() {
        let ahead = STALL_FRAMES; // 150 ms
        assert_eq!(silence_needed(1000, 1000), 0, "al día");
        assert_eq!(silence_needed(1000, 1000 + 480), 0, "un retraso normal de 10 ms no se rellena");
        assert_eq!(silence_needed(1000, 1000 + ahead), 0, "justo en el límite");
        assert_eq!(silence_needed(1000, 1000 + ahead + 1), ahead + 1, "captura detenida: se rellena hasta el reloj");
        assert_eq!(silence_needed(5000, 1000), 0, "si el audio va por delante, no se añade nada");
    }

    #[test]
    fn the_queue_cannot_grow_without_limit() {
        let queue = new_queue();
        let mut local: VecDeque<u8> = std::iter::repeat_n(1u8, MAX_QUEUED_BYTES * 3).collect();
        push(&queue, &mut local);
        let len = queue.lock().unwrap().len();
        assert!(len <= MAX_QUEUED_BYTES && len % FRAME_BYTES == 0, "{len}");
    }

    #[tokio::test]
    async fn pcm_reaches_a_pipe_client_in_real_time_and_closes_cleanly() {
        use tokio::io::AsyncReadExt;
        let name = format!(r"\\.\pipe\strata-test-{}", uuid::Uuid::new_v4());
        let server = create_pipe(&name).unwrap();
        let queue = new_queue();
        let stop = Arc::new(AtomicBool::new(false));
        let writer = tokio::spawn(write_pcm(server, queue.clone(), stop.clone()));
        // FFmpeg abre la tubería en solo lectura; la del test debe hacer lo mismo.
        let mut client = tokio::net::windows::named_pipe::ClientOptions::new().read(true).write(false).open(&name).unwrap();
        // Lo capturado antes de la conexión se descarta; lo que llega después se envía enseguida.
        queue.lock().unwrap().extend(vec![9u8; 960 * FRAME_BYTES]);
        tokio::time::sleep(Duration::from_millis(100)).await;
        queue.lock().unwrap().extend(vec![5u8; 960 * FRAME_BYTES]);
        let mut got = vec![0u8; 48_000 * FRAME_BYTES / 4]; // 0,25 s
        tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut got)).await.unwrap().unwrap();
        assert!(got.iter().any(|b| *b == 5), "llega el audio capturado después de conectar");
        assert!(!got.iter().any(|b| *b == 9), "lo anterior a la conexión no llega");
        stop.store(true, Ordering::Relaxed);
        writer.await.unwrap().unwrap();
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut rest)).await.unwrap().unwrap();
    }
}
