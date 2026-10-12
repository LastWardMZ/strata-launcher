//! Construcción de las líneas de comando de FFmpeg (grabar, probar codificadores y recortar).
//! Son funciones puras: no ejecutan nada, así que se pueden comprobar con pruebas unitarias.

use std::path::Path;

/// Codificadores H.264 en orden de preferencia: primero los de hardware, por último el de software.
pub const ENCODER_CANDIDATES: [&str; 5] = ["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf", "libx264"];

pub fn is_hardware(encoder: &str) -> bool {
    encoder != "libx264"
}

pub fn encoder_label(encoder: &str) -> &'static str {
    match encoder {
        "h264_nvenc" => "NVIDIA NVENC",
        "h264_amf" => "AMD AMF",
        "h264_qsv" => "Intel Quick Sync",
        "h264_mf" => "Media Foundation (hardware)",
        _ => "Software (x264)",
    }
}

/// Tamaño de salida: nunca se amplía y siempre es par (lo exige el vídeo YUV 4:2:0).
pub fn target_size(src_w: u32, src_h: u32, resolution: &str) -> (u32, u32) {
    let cap = match resolution {
        "1080p" => 1080,
        "720p" => 720,
        _ => u32::MAX,
    };
    let (mut w, mut h) = (src_w.max(2), src_h.max(2));
    if h > cap {
        w = ((f64::from(w) * f64::from(cap)) / f64::from(h)).round() as u32;
        h = cap;
    }
    ((w & !1).max(2), (h & !1).max(2))
}

/// Bits por segundo objetivo (para los codificadores que trabajan por tasa de bits).
pub fn target_bitrate(quality: &str, width: u32, height: u32, fps: u32) -> u32 {
    let bits_per_pixel = match quality {
        "low" => 0.04,
        "high" => 0.11,
        _ => 0.07,
    };
    let rate = f64::from(width) * f64::from(height) * f64::from(fps) * bits_per_pixel;
    (rate as u32).clamp(2_000_000, 40_000_000)
}

/// Parámetros del codificador de vídeo. La clave es un fotograma clave cada 2 s: así el recorte rápido
/// (sin recodificar) tiene como mucho 2 s de margen y los fragmentos de recuperación son pequeños.
pub fn video_codec_args(encoder: &str, quality: &str, width: u32, height: u32, fps: u32) -> Vec<String> {
    let gop = (fps * 2).to_string();
    let bitrate = target_bitrate(quality, width, height, fps);
    let bitrate_k = format!("{}k", bitrate / 1000);
    let max_k = format!("{}k", bitrate * 3 / 2000);
    let strings = |items: &[&str]| items.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    match encoder {
        "libx264" => {
            let crf = match quality {
                "low" => "28",
                "high" => "19",
                _ => "23",
            };
            strings(&["-c:v", "libx264", "-preset", "veryfast", "-crf", crf, "-g", &gop, "-pix_fmt", "yuv420p"])
        }
        "h264_nvenc" => {
            let cq = match quality {
                "low" => "30",
                "high" => "21",
                _ => "25",
            };
            strings(&[
                "-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", cq, "-b:v", &bitrate_k, "-maxrate", &max_k,
                "-g", &gop, "-pix_fmt", "yuv420p",
            ])
        }
        "h264_amf" => strings(&[
            "-c:v", "h264_amf", "-quality", "speed", "-rc", "vbr_peak", "-b:v", &bitrate_k, "-maxrate", &max_k, "-g",
            &gop, "-pix_fmt", "yuv420p",
        ]),
        "h264_qsv" => strings(&["-c:v", "h264_qsv", "-preset", "veryfast", "-b:v", &bitrate_k, "-g", &gop, "-pix_fmt", "nv12"]),
        "h264_mf" => {
            let q = match quality {
                "low" => "45",
                "high" => "85",
                _ => "65",
            };
            strings(&["-c:v", "h264_mf", "-hw_encoding", "1", "-rate_control", "quality", "-quality", q, "-g", &gop, "-pix_fmt", "nv12"])
        }
        other => strings(&["-c:v", other]),
    }
}

/// Fotogramas por segundo máximos que acepta la captura de ventana (ver `video_filter`).
pub const CAPTURE_FRAMERATE_CAP: u32 = 240;

/// Describe qué se captura y a qué tamaño.
pub struct CaptureSpec {
    pub hwnd: i64,
    pub fps: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub dst_w: u32,
    pub dst_h: u32,
    /// Instante (µs desde 1970) que es el «cero» de las marcas de tiempo de vídeo y audio. Lo fija el launcher justo antes
    /// de lanzar FFmpeg para que las dos pistas compartan exactamente el mismo origen (cada filtro de FFmpeg fija su
    /// propio `RTCSTART` al configurarse y `gfxcapture` tarda ~85 ms más que el audio: eso desfasaba las pistas).
    pub clock_origin_us: u64,
}

/// Grafo de vídeo: captura de la ventana (Windows.Graphics.Capture) → FPS constantes → memoria del
/// sistema → matriz de color BT.709 → NV12.
pub fn video_filter(spec: &CaptureSpec) -> String {
    // El tope de la captura va muy por encima de lo que produce cualquier ventana (Minecraft puede pasar de 200 fps) y `fps`
    // descarta los fotogramas sobrantes en la GPU, antes de leerlos a memoria. Con un tope bajo, la captura consume menos
    // de lo que la ventana produce, su cola interna se queda siempre llena y entrega los fotogramas más antiguos primero:
    // la imagen llegaba con una demora fija de ~200 ms (y distinta de una grabación a otra).
    let mut capture = format!("gfxcapture=hwnd=0x{:X}:max_framerate={CAPTURE_FRAMERATE_CAP}:capture_cursor=1", spec.hwnd);
    if (spec.dst_w, spec.dst_h) != (spec.src_w, spec.src_h) {
        capture.push_str(&format!(":width={}:height={}:resize_mode=scale:scale_mode=bilinear", spec.dst_w, spec.dst_h));
    }
    format!(
        "{capture},{},{},hwdownload,format=bgra,scale=flags=bilinear:out_color_matrix=bt709:out_range=tv,format=nv12",
        wall_clock_video(spec.clock_origin_us),
        frame_pacing(spec.fps)
    )
}

/// Ritmo de fotogramas: tasa constante. Se probó conservar la hora real de cada fotograma (tasa variable, sin `fps`), pero
/// no repetía menos fotogramas (7,9 % frente a 7,3 % con un juego a 72 fps en una pantalla de 60 Hz) y, junto con el
/// audio, el vídeo caía a ~20 fps: sin el filtro `fps` el grafo con audio frena la captura.
fn frame_pacing(fps: u32) -> String {
    format!("fps={fps}")
}

/// Marca de tiempo = segundos de reloj de pared desde `origin_us`. Se aplica por igual al vídeo (`setpts`) y al audio
/// (`asetpts`): así las dos pistas comparten referencia y quedan sincronizadas aunque lleguen por caminos distintos.
fn wall_clock_video(origin_us: u64) -> String {
    format!("setpts=(RTCTIME-{origin_us})/(TB*1000000)")
}

/// Retraso (en segundos) que añade al audio el propio recorrido hasta FFmpeg: escritor, tubería, empaquetado de PCM,
/// marca de tiempo al llegar y codificador AAC. Se midió con una fuente sintética precisa y una referencia de vídeo
/// generada por FFmpeg (sin latencia de captura): 82–122 ms de forma estable, sin depender del equipo ni de la tarjeta
/// de sonido, que quedan fuera (son parte de lo que el jugador oye realmente). Se resta de la marca de tiempo del audio.
pub const AUDIO_PIPELINE_DELAY: f64 = 0.100;

fn wall_clock_audio(origin_us: u64) -> String {
    format!("asetpts=(RTCTIME-{origin_us})/(TB*1000000)-{AUDIO_PIPELINE_DELAY:.3}/TB")
}

/// Vídeo y audio se graban en procesos de FFmpeg distintos y se unen al terminar (sin recodificar). Con las dos pistas en el
/// mismo proceso, el audio frenaba la captura: con un juego a ~65 fps entraban ~25 fotogramas distintos por segundo en
/// lugar de ~50 (el filtro `fps` rellenaba el resto repitiendo imágenes, y el vídeo se veía a tirones aunque dijera 60 fps).
/// Ambos procesos sellan sus marcas de tiempo con el mismo origen de reloj, así que la unión conserva la sincronía.
pub struct VideoArgs<'a> {
    pub spec: &'a CaptureSpec,
    pub encoder: &'a str,
    pub quality: &'a str,
    /// Archivo temporal de salida.
    pub output: &'a Path,
}

pub struct AudioArgs<'a> {
    /// Mismo origen de reloj que el vídeo (`CaptureSpec::clock_origin_us`).
    pub clock_origin_us: u64,
    /// Tubería con el audio del juego (PCM f32 estéreo a 48 kHz).
    pub game_pipe: Option<&'a str>,
    /// Tubería con el micrófono (mismo formato).
    pub mic_pipe: Option<&'a str>,
    pub output: &'a Path,
}

pub fn container_format(format: &str) -> &'static str {
    if format == "mkv" { "matroska" } else { "mp4" }
}

/// Proceso de vídeo: solo captura de ventana y codificación. Los temporales son siempre Matroska: aguantan un cierre
/// inesperado y, a diferencia de MP4, conservan la hora absoluta del primer fotograma (necesaria para sincronizar con el
/// audio al unirlos).
pub fn video_recording_args(a: &VideoArgs) -> Vec<String> {
    // `-n`: jamás sobrescribir. `-stats_period`: progreso frecuente (se lee para saber que ya se graba).
    let mut args: Vec<String> = ["-hide_banner", "-n", "-stats_period", "0.5"].map(String::from).to_vec();
    args.extend(["-filter_complex".to_string(), format!("{}[v]", video_filter(a.spec)), "-map".to_string(), "[v]".to_string()]);
    args.extend(video_codec_args(a.encoder, a.quality, a.spec.dst_w, a.spec.dst_h, a.spec.fps));
    args.extend(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"].map(String::from));
    args.extend(["-f", "matroska"].map(String::from));
    args.push(a.output.to_string_lossy().into_owned());
    args
}

/// Proceso de audio: mezcla el audio del juego y el micrófono (si hay) y los codifica. Sin entradas no tiene sentido
/// lanzarlo: devuelve `None`.
pub fn audio_recording_args(a: &AudioArgs) -> Option<Vec<String>> {
    let mut args: Vec<String> = ["-hide_banner", "-n", "-stats_period", "0.5"].map(String::from).to_vec();
    let mut inputs = 0;
    for pipe in [a.game_pipe, a.mic_pipe].into_iter().flatten() {
        // Sin `-use_wallclock_as_timestamps`: daba la hora absoluta del reloj y el remuestreador se atascaba
        // intentando compensar décadas de diferencia con el vídeo. La referencia común se fija en el grafo.
        args.extend(["-thread_queue_size", "1024", "-f", "f32le", "-ar", "48000", "-ac", "2", "-i"].map(String::from));
        args.push(pipe.to_string());
        inputs += 1;
    }
    let clock = wall_clock_audio(a.clock_origin_us);
    let graph = match inputs {
        1 => format!("[0:a]{clock},aresample=async=1:first_pts=0[a]"),
        2 => format!("[0:a]{clock}[a0];[1:a]{clock}[a1];[a0][a1]amix=inputs=2:duration=longest:normalize=0,aresample=async=1:first_pts=0[a]"),
        _ => return None,
    };
    args.extend(["-filter_complex".to_string(), graph, "-map".to_string(), "[a]".to_string()]);
    args.extend(["-c:a", "aac", "-b:a", "160k", "-f", "matroska"].map(String::from));
    args.push(a.output.to_string_lossy().into_owned());
    Some(args)
}

/// Prueba rápida de un codificador: codifica 3 fotogramas de un vídeo sintético y lo descarta.
pub fn probe_encoder_args(encoder: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=256x256:r=30"]
        .map(String::from)
        .to_vec();
    args.extend(["-frames:v", "3", "-pix_fmt", "nv12"].map(String::from));
    args.extend(video_codec_args(encoder, "medium", 256, 256, 30).into_iter().filter(|a| a != "-pix_fmt" && a != "yuv420p" && a != "nv12"));
    args.extend(["-f", "null", "-"].map(String::from));
    args
}

/// Une el vídeo temporal (y el audio, si lo hay) en el archivo final sin recodificar: índice al principio y duración
/// conocida. `-copyts` conserva la hora absoluta de cada pista (el vídeo empieza unas décimas después que el audio) y
/// `make_zero` desplaza las dos por igual para que el archivo empiece en 0 sin perder su sincronía.
pub fn finalize_args(video: &Path, audio: Option<&Path>, output: &Path, format: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-n", "-copyts", "-i"].map(String::from).to_vec();
    args.push(video.to_string_lossy().into_owned());
    if let Some(audio) = audio {
        args.extend(["-i".to_string(), audio.to_string_lossy().into_owned()]);
        args.extend(["-map", "0:v:0", "-map", "1:a:0"].map(String::from));
    } else {
        args.extend(["-map", "0:v:0", "-map", "0:a?"].map(String::from));
    }
    args.extend(["-c", "copy", "-avoid_negative_ts", "make_zero"].map(String::from));
    args.extend(["-f".to_string(), container_format(format).to_string()]);
    if format != "mkv" {
        args.extend(["-movflags", "+faststart"].map(String::from));
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrimMode {
    /// Corte exacto: se recodifica (más lento).
    Precise,
    /// Sin recodificar: instantáneo, pero el inicio cae en el fotograma clave anterior (hasta ~2 s antes).
    Fast,
}

/// Datos del vídeo original que necesita un recorte con recodificación.
pub struct TrimSource<'a> {
    pub encoder: &'a str,
    pub quality: &'a str,
    pub size: (u32, u32),
    pub fps: u32,
    /// `mp4` o `mkv`: el del archivo original.
    pub format: &'a str,
    /// Velocidad de reproducción del resultado (1.0 = normal). Distinta de 1 exige recodificar.
    pub speed: f64,
    /// Quitar la pista de audio.
    pub mute: bool,
}

/// Velocidades que ofrece el editor (el filtro de audio `atempo` admite de 0.5 a 2).
pub const TRIM_SPEEDS: [f64; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];

pub fn trim_args(input: &Path, output: &Path, start: f64, end: f64, mode: TrimMode, source: &TrimSource) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-n", "-progress", "pipe:1"].map(String::from).to_vec();
    // `-ss` antes de `-i` es rápido y, al recodificar, también exacto.
    // `-t` va antes de `-i`: limita lo que se LEE del original, así que con otra velocidad la salida sale entera.
    args.extend(["-ss".to_string(), format!("{start:.3}"), "-t".to_string(), format!("{:.3}", (end - start).max(0.0))]);
    args.extend(["-i".to_string(), input.to_string_lossy().into_owned()]);
    args.extend(["-map", "0:v:0", "-map", "0:a?"].map(String::from));
    let changes_speed = (source.speed - 1.0).abs() > f64::EPSILON;
    match mode {
        TrimMode::Fast if !changes_speed => {
            args.extend(["-c", "copy", "-avoid_negative_ts", "make_zero"].map(String::from));
            if source.mute {
                args.push("-an".into());
            }
        }
        _ => {
            if changes_speed {
                args.extend(["-filter:v".to_string(), format!("setpts=PTS/{}", source.speed)]);
                if !source.mute {
                    args.extend(["-filter:a".to_string(), format!("atempo={}", source.speed)]);
                }
            }
            args.extend(video_codec_args(source.encoder, source.quality, source.size.0, source.size.1, source.fps));
            if source.mute {
                args.push("-an".into());
            } else {
                args.extend(["-c:a", "aac", "-b:a", "160k"].map(String::from));
            }
        }
    }
    args.extend(["-f".to_string(), container_format(source.format).to_string()]);
    if source.format != "mkv" {
        args.extend(["-movflags", "+faststart"].map(String::from));
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Un vídeo que entra en una unión.
pub struct MergeInput<'a> {
    pub path: &'a Path,
    pub duration: f64,
    pub has_audio: bool,
}

/// Une los vídeos en orden con `concat`: todos se escalan al tamaño del primero (con bandas si cambia la proporción), a
/// sus FPS y a audio estéreo de 48 kHz; a los que no tienen sonido se les añade silencio de su misma duración.
pub fn merge_args(inputs: &[MergeInput], output: &Path, size: (u32, u32), fps: u32, encoder: &str, quality: &str, format: &str) -> Vec<String> {
    let (w, h) = size;
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-n", "-progress", "pipe:1"].map(String::from).to_vec();
    for input in inputs {
        args.extend(["-i".to_string(), input.path.to_string_lossy().into_owned()]);
    }
    let mut filters: Vec<String> = Vec::new();
    let mut next_silence = inputs.len();
    for (i, input) in inputs.iter().enumerate() {
        filters.push(format!(
            "[{i}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps},format=yuv420p[v{i}]"
        ));
        if input.has_audio {
            filters.push(format!("[{i}:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}]"));
        } else {
            args.extend(["-f".to_string(), "lavfi".to_string(), "-t".to_string(), format!("{:.3}", input.duration), "-i".to_string(), "anullsrc=r=48000:cl=stereo".to_string()]);
            filters.push(format!("[{next_silence}:a:0]aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}]"));
            next_silence += 1;
        }
    }
    let pairs: String = (0..inputs.len()).map(|i| format!("[v{i}][a{i}]")).collect();
    filters.push(format!("{pairs}concat=n={}:v=1:a=1[v][a]", inputs.len()));
    args.extend(["-filter_complex".to_string(), filters.join(";")]);
    args.extend(["-map", "[v]", "-map", "[a]"].map(String::from));
    args.extend(video_codec_args(encoder, quality, w, h, fps));
    args.extend(["-c:a", "aac", "-b:a", "160k"].map(String::from));
    args.extend(["-f".to_string(), container_format(format).to_string()]);
    if format != "mkv" {
        args.extend(["-movflags", "+faststart"].map(String::from));
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Fragmento como GIF animado: 15 fotogramas por segundo, 480 px de ancho y paleta propia (mucha mejor calidad que la fija).
pub fn gif_args(input: &Path, output: &Path, start: f64, end: f64) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-n", "-progress", "pipe:1"].map(String::from).to_vec();
    args.extend(["-ss".to_string(), format!("{start:.3}"), "-i".to_string(), input.to_string_lossy().into_owned()]);
    args.extend(["-t".to_string(), format!("{:.3}", (end - start).max(0.0))]);
    args.extend(["-an", "-vf"].map(String::from));
    args.push("fps=15,scale=480:-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=4".into());
    args.extend(["-loop", "0", "-f", "gif"].map(String::from));
    args.push(output.to_string_lossy().into_owned());
    args
}

pub fn thumbnail_args(input: &Path, output: &Path, at_seconds: f64) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-ss"].map(String::from).to_vec();
    args.push(format!("{at_seconds:.2}"));
    args.push("-i".into());
    args.push(input.to_string_lossy().into_owned());
    args.extend(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "5"].map(String::from));
    args.push(output.to_string_lossy().into_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(dst: (u32, u32)) -> CaptureSpec {
        CaptureSpec { hwnd: 0x1089A, fps: 60, src_w: 1920, src_h: 1080, dst_w: dst.0, dst_h: dst.1, clock_origin_us: 1_791_724_818_000_000 }
    }

    #[test]
    fn sizes_are_even_and_never_upscaled() {
        assert_eq!(target_size(1920, 1080, "original"), (1920, 1080));
        assert_eq!(target_size(1921, 1081, "original"), (1920, 1080));
        assert_eq!(target_size(1920, 1080, "720p"), (1280, 720));
        assert_eq!(target_size(1280, 720, "1080p"), (1280, 720), "no se amplía");
        assert_eq!(target_size(2560, 1440, "1080p"), (1920, 1080));
        assert_eq!(target_size(854, 480, "720p"), (854, 480));
        assert_eq!(target_size(1, 1, "original"), (2, 2));
    }

    #[test]
    fn bitrate_scales_with_quality_and_is_clamped() {
        let low = target_bitrate("low", 1920, 1080, 30);
        let medium = target_bitrate("medium", 1920, 1080, 30);
        let high = target_bitrate("high", 1920, 1080, 30);
        assert!(low < medium && medium < high);
        assert_eq!(target_bitrate("low", 320, 240, 30), 2_000_000);
        assert_eq!(target_bitrate("high", 7680, 4320, 60), 40_000_000);
    }

    #[test]
    fn every_encoder_gets_a_keyframe_every_two_seconds() {
        for encoder in ENCODER_CANDIDATES {
            let args = video_codec_args(encoder, "medium", 1920, 1080, 60);
            let g = args.iter().position(|a| a == "-g").expect("lleva -g");
            assert_eq!(args[g + 1], "120", "{encoder}");
            assert_eq!(args[1], encoder);
        }
        assert!(is_hardware("h264_nvenc") && !is_hardware("libx264"));
    }

    #[test]
    fn filter_graph_captures_the_window_and_only_scales_when_needed() {
        let native = video_filter(&spec((1920, 1080)));
        assert!(
            native.starts_with("gfxcapture=hwnd=0x1089A:max_framerate=240:capture_cursor=1,setpts=(RTCTIME-1791724818000000)/(TB*1000000),fps=60,hwdownload"),
            "{native}"
        );
        assert!(!native.contains("width="), "sin reescalado: {native}");
        assert!(native.contains("out_color_matrix=bt709") && native.ends_with("format=nv12"));
        let scaled = video_filter(&spec((1280, 720)));
        assert!(scaled.contains(":width=1280:height=720:resize_mode=scale"), "{scaled}");
    }

    #[test]
    fn video_and_audio_run_in_separate_processes_sharing_one_clock() {
        let output = PathBuf::from(r"C:\v\a.mp4.partial");
        let video = video_recording_args(&VideoArgs { spec: &spec((1920, 1080)), encoder: "libx264", quality: "medium", output: &output }).join(" ");
        assert!(video.contains("-n ") && video.contains("-f matroska") && video.ends_with(r"C:\v\a.mp4.partial"));
        assert!(!video.contains("-i ") && !video.contains("[a]") && !video.contains("-c:a"), "el proceso de vídeo no lleva audio: {video}");
        assert!(video.contains("setpts=(RTCTIME-1791724818000000)/(TB*1000000)"), "el vídeo sella con el origen común");

        let audio_out = PathBuf::from(r"C:\v\a.mp4.partial-audio");
        let both = audio_recording_args(&AudioArgs {
            clock_origin_us: 1_791_724_818_000_000,
            game_pipe: Some(r"\\.\pipe\g"),
            mic_pipe: Some(r"\\.\pipe\m"),
            output: &audio_out,
        })
        .expect("hay entradas")
        .join(" ");
        assert!(both.contains("-n ") && both.contains(r"-i \\.\pipe\g") && both.contains(r"-i \\.\pipe\m"));
        assert!(both.contains("amix=inputs=2:duration=longest:normalize=0"));
        assert!(!both.contains("use_wallclock_as_timestamps"), "los pts absolutos del reloj rompen el remuestreo");
        assert_eq!(
            both.matches("asetpts=(RTCTIME-1791724818000000)/(TB*1000000)-0.100/TB").count(),
            2,
            "ambas pistas con el mismo origen que el vídeo y la compensación del recorrido interno"
        );
        assert!(!both.contains("RTCSTART"), "RTCSTART difiere entre filtros: no se usa");
        assert!(both.contains("-map [a] -c:a aac") && !both.contains("gfxcapture") && both.ends_with(r"C:\v\a.mp4.partial-audio"));

        let one = audio_recording_args(&AudioArgs { clock_origin_us: 1_791_724_818_000_000, game_pipe: Some("x"), mic_pipe: None, output: &audio_out })
            .unwrap()
            .join(" ");
        assert!(one.contains("[0:a]asetpts=(RTCTIME-1791724818000000)/(TB*1000000)-0.100/TB,aresample=async=1:first_pts=0[a]"));
        assert!(
            audio_recording_args(&AudioArgs { clock_origin_us: 1, game_pipe: None, mic_pipe: None, output: &audio_out }).is_none(),
            "sin entradas no hay proceso de audio"
        );
    }

    #[test]
    fn finalize_joins_tracks_without_losing_their_offset() {
        let with_audio = finalize_args(Path::new("a.mp4.partial"), Some(Path::new("a.mp4.partial-audio")), Path::new("a.mp4"), "mp4").join(" ");
        assert!(with_audio.contains("-copyts -i a.mp4.partial -i a.mp4.partial-audio"), "{with_audio}");
        assert!(with_audio.contains("-map 0:v:0 -map 1:a:0 -c copy -avoid_negative_ts make_zero") && with_audio.contains("+faststart"));
        let silent = finalize_args(Path::new("a.mkv.partial"), None, Path::new("a.mkv"), "mkv").join(" ");
        assert!(silent.contains("-map 0:v:0 -map 0:a?") && silent.contains("-f matroska") && !silent.contains("faststart"), "{silent}");
    }

    #[test]
    fn encoder_probe_is_a_tiny_synthetic_encode() {
        for encoder in ENCODER_CANDIDATES {
            let args = probe_encoder_args(encoder).join(" ");
            assert!(args.contains("color=c=black:s=256x256") && args.contains("-frames:v 3"));
            assert!(args.ends_with("-f null -") && args.contains(encoder));
            assert_eq!(args.matches("-pix_fmt").count(), 1, "un único formato de píxel: {args}");
        }
    }

    #[test]
    fn speed_and_mute_force_reencoding_and_keep_the_whole_output() {
        let (i, o) = (PathBuf::from("in.mp4"), PathBuf::from("out.mp4"));
        let base = TrimSource { encoder: "libx264", quality: "medium", size: (1920, 1080), fps: 30, format: "mp4", speed: 1.0, mute: false };
        // El límite de duración se aplica a la entrada (antes de -i), no a la salida.
        let normal = trim_args(&i, &o, 10.0, 20.0, TrimMode::Precise, &base);
        let t = normal.iter().position(|a| a == "-t").unwrap();
        let input = normal.iter().position(|a| a == "-i").unwrap();
        assert!(t < input, "-t va antes de -i");

        // Más lento: se recodifica aunque se pida «rápido», con vídeo y audio al mismo ritmo.
        let slow = trim_args(&i, &o, 10.0, 20.0, TrimMode::Fast, &TrimSource { speed: 0.5, ..base }).join(" ");
        assert!(slow.contains("setpts=PTS/0.5") && slow.contains("atempo=0.5"), "{slow}");
        assert!(slow.contains("-c:v libx264") && !slow.contains("-c copy"));

        // Silenciar un corte rápido conserva la copia directa del vídeo y quita el audio.
        let muted = trim_args(&i, &o, 10.0, 20.0, TrimMode::Fast, &TrimSource { mute: true, ..base }).join(" ");
        assert!(muted.contains("-c copy") && muted.contains("-an"));

        // Más rápido y sin sonido: sin filtro de audio ni codificación de audio.
        let fast_mute = trim_args(&i, &o, 10.0, 20.0, TrimMode::Precise, &TrimSource { speed: 2.0, mute: true, ..base }).join(" ");
        assert!(fast_mute.contains("setpts=PTS/2") && !fast_mute.contains("atempo") && !fast_mute.contains("-c:a") && fast_mute.contains("-an"));
        assert!(TRIM_SPEEDS.iter().all(|s| (0.5..=2.0).contains(s)));
    }

    #[test]
    fn merge_scales_everything_to_the_first_video_and_fills_missing_audio() {
        let a = PathBuf::from("a.mp4");
        let b = PathBuf::from("b.mp4");
        let c = PathBuf::from("c.mp4");
        let inputs = [
            MergeInput { path: &a, duration: 10.0, has_audio: true },
            MergeInput { path: &b, duration: 4.5, has_audio: false },
            MergeInput { path: &c, duration: 2.0, has_audio: true },
        ];
        let args = merge_args(&inputs, &PathBuf::from("out.mp4.trimming"), (1920, 1080), 60, "libx264", "medium", "mp4").join(" ");
        assert!(args.contains("-i a.mp4 -i b.mp4 -i c.mp4 -f lavfi -t 4.500 -i anullsrc"), "el silencio va después de los vídeos: {args}");
        assert!(args.contains("[1:v:0]scale=1920:1080"), "cada vídeo se escala al del primero");
        assert!(args.contains("[3:a:0]aformat") && args.contains("[a1]"), "el vídeo sin sonido usa el silencio");
        assert!(args.contains("[2:a:0]aresample") && !args.contains("[1:a:0]"));
        assert!(args.contains("[v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[v][a]"));
        assert!(args.contains("-map [v] -map [a]") && args.contains("-f mp4"));
    }

    #[test]
    fn gif_export_uses_a_custom_palette_and_no_audio() {
        let args = gif_args(&PathBuf::from("in.mp4"), &PathBuf::from("out.gif.trimming"), 5.0, 12.5).join(" ");
        assert!(args.contains("-ss 5.000") && args.contains("-t 7.500"));
        assert!(args.contains("-an"), "un GIF no lleva audio");
        assert!(args.contains("palettegen") && args.contains("paletteuse"));
        assert!(args.contains("-f gif"), "formato explícito: el temporal no acaba en .gif");
        assert!(args.contains("fps=15") && args.contains("scale=480:-2"));
    }

    #[test]
    fn trim_modes_copy_or_reencode() {
        let (i, o) = (PathBuf::from("in.mp4"), PathBuf::from("out.mp4"));
        let source = TrimSource { encoder: "libx264", quality: "medium", size: (1920, 1080), fps: 30, format: "mp4", speed: 1.0, mute: false };
        let fast = trim_args(&i, &o, 35.0, 58.0, TrimMode::Fast, &source).join(" ");
        assert!(fast.contains("-ss 35.000 -t 23.000 -i in.mp4") && fast.contains("-c copy"), "{fast}");
        let precise = trim_args(&i, &o, 35.0, 58.0, TrimMode::Precise, &source).join(" ");
        assert!(precise.contains("-c:v libx264") && precise.contains("-c:a aac") && !precise.contains("-c copy"));
        assert!(precise.contains("-n ") && precise.contains("-progress pipe:1"), "no sobrescribe y da progreso");
        assert!(precise.contains("-t 23.000"), "duración = fin − inicio");
    }

    #[test]
    fn finalize_and_thumbnail_commands() {
        let f = finalize_args(Path::new("a.mp4.partial"), None, Path::new("a.mp4"), "mp4").join(" ");
        assert!(f.contains("-c copy") && f.contains("+faststart") && f.contains("-n"));
        let t = thumbnail_args(Path::new("a.mp4"), Path::new("a.jpg"), 1.5).join(" ");
        assert!(t.contains("-ss 1.50") && t.contains("scale=480:-2"));
    }
}
