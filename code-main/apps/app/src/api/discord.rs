//! Nivel de detalle de Discord Rich Presence (completo o básico). Apagarlo del todo es el ajuste de siempre.
use crate::api::Result;
use theseus::{DiscordDetail, discord_detail, set_discord_detail};

#[tauri::command]
pub async fn discord_detail_get() -> Result<String> {
    Ok(discord_detail().await?.as_str().to_string())
}

#[tauri::command]
pub async fn discord_detail_set(level: String) -> Result<()> {
    let detail = DiscordDetail::parse(&level).ok_or_else(|| theseus::ErrorKind::OtherError("Nivel de Discord no válido".into()).as_error())?;
    Ok(set_discord_detail(detail).await?)
}
