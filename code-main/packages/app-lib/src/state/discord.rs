use std::sync::{Arc, atomic::AtomicBool};

use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, Assets, Timestamps},
};
use tokio::sync::RwLock;

use crate::State;

/// Aplicación «Strata Minecraft» registrada en el Developer Portal de Discord. El nombre que Discord
/// muestra («Jugando a …») sale de esta aplicación, no del código.
const DISCORD_APPLICATION_ID: &str = "1559276683973107873";
/// Clave de la imagen grande, subida en Rich Presence → Art Assets de esa aplicación.
const DISCORD_LARGE_IMAGE: &str = "strata";

/// Cuánto muestra Discord de lo que se juega. Se guarda en un archivo de la carpeta de ajustes (no en la base de datos).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiscordDetail {
    /// «Jugando a {instancia}» y «Minecraft {versión} · {cargador}».
    Full,
    /// Solo «Jugando a Minecraft»: sin nombre de instancia ni versiones.
    Basic,
}

impl DiscordDetail {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Basic => "basic",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "full" => Some(Self::Full),
            "basic" => Some(Self::Basic),
            _ => None,
        }
    }
}

const DETAIL_FILE: &str = "discord-detail.txt";

/// Nivel guardado (por defecto, completo).
pub async fn discord_detail() -> crate::Result<DiscordDetail> {
    let state = State::get().await?;
    let path = state.directories.settings_dir.join(DETAIL_FILE);
    Ok(std::fs::read_to_string(path).ok().and_then(|text| DiscordDetail::parse(&text)).unwrap_or(DiscordDetail::Full))
}

/// Guarda el nivel y actualiza la presencia ya mostrada.
pub async fn set_discord_detail(detail: DiscordDetail) -> crate::Result<()> {
    let state = State::get().await?;
    std::fs::write(state.directories.settings_dir.join(DETAIL_FILE), detail.as_str())?;
    let _ = state.discord_rpc.refresh().await;
    Ok(())
}

pub struct DiscordGuard {
    client: Arc<RwLock<DiscordIpcClient>>,
    connected: Arc<AtomicBool>,
}

impl DiscordGuard {
    /// Initialize discord IPC client, and attempt to connect to it
    /// If it fails, it will still return a DiscordGuard, but the client will be unconnected
    pub fn init() -> crate::Result<DiscordGuard> {
        let dipc = DiscordIpcClient::new(DISCORD_APPLICATION_ID);

        Ok(DiscordGuard {
            client: Arc::new(RwLock::new(dipc)),
            connected: Arc::new(AtomicBool::new(false)),
        })
    }

    /// If the client failed connecting during init(), this will check for connection and attempt to reconnect
    /// This MUST be called first in any client method that requires a connection, because those can PANIC if the client is not connected
    /// (No connection is different than a failed connection, the latter will not panic and can be retried)
    pub async fn retry_if_not_ready(&self) -> bool {
        let mut client = self.client.write().await;
        if !self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            if client.connect().is_ok() {
                self.connected
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                return true;
            }
            return false;
        }
        true
    }

    /// Set the activity to the given message
    /// First checks if discord is disabled, and if so, clear the activity instead
    pub async fn set_activity(
        &self,
        details: &str,
        status: Option<&str>,
        reconnect_if_fail: bool,
    ) -> crate::Result<()> {
        // Check if discord is disabled, and if so, clear the activity instead
        let state = State::get().await?;
        let settings = crate::state::Settings::get(&state.pool).await?;
        if !settings.discord_rpc {
            Ok(self.clear_activity(true).await?)
        } else {
            Ok(self.force_set_activity(details, status, reconnect_if_fail).await?)
        }
    }

    /// Sets the activity to the given message, regardless of if discord is disabled or offline
    /// Should not be used except for in the above method, or if it is already known that discord is enabled (specifically for state initialization) and we are connected to the internet
    pub async fn force_set_activity(
        &self,
        details: &str,
        status: Option<&str>,
        reconnect_if_fail: bool,
    ) -> crate::Result<()> {
        // Attempt to connect if not connected. Do not continue if it fails, as the client.set_activity can panic if it never was connected
        if !self.retry_if_not_ready().await {
            return Ok(());
        }

        // En el nivel básico no sale el nombre de la instancia ni la versión.
        let basic = discord_detail().await.map(|level| level == DiscordDetail::Basic).unwrap_or(false);
        let (details, status) = if basic && details.starts_with("Jugando a ") { ("Jugando a Minecraft", None) } else { (details, status) };

        let mut activity = Activity::new()
            .details(details)
            .timestamps(Timestamps::new().start(chrono::Utc::now().timestamp()))
            .assets(
                Assets::new()
                    .large_image(DISCORD_LARGE_IMAGE)
                    .large_text("Strata Minecraft"),
            );
        if let Some(status) = status {
            activity = activity.state(status);
        }

        // Attempt to set the activity
        // If the existing connection fails, attempt to reconnect and try again
        let mut client: tokio::sync::RwLockWriteGuard<'_, DiscordIpcClient> =
            self.client.write().await;
        let res = client.set_activity(activity.clone());

        if reconnect_if_fail {
            if let Err(_e) = res {
                client.reconnect()?;
                return Ok(client.set_activity(activity)?); // try again, but don't reconnect if it fails again
            }
        } else {
            res?;
        }

        Ok(())
    }

    /// Clear the activity entirely ('disabling' the RPC until the next set_activity)
    pub async fn clear_activity(
        &self,
        reconnect_if_fail: bool,
    ) -> crate::Result<()> {
        // Attempt to connect if not connected. Do not continue if it fails, as the client.clear_activity can panic if it never was connected
        if !self.retry_if_not_ready().await {
            return Ok(());
        }

        // Attempt to clear the activity
        // If the existing connection fails, attempt to reconnect and try again
        let mut client = self.client.write().await;
        let res = client.clear_activity();

        if reconnect_if_fail {
            if res.is_err() {
                client.reconnect()?;
                return Ok(client.clear_activity()?); // try again, but don't reconnect if it fails again
            }
        } else {
            res?;
        }
        Ok(())
    }

    /// Vuelve a calcular y mostrar la presencia actual (p. ej. tras cambiar el nivel de detalle).
    pub async fn refresh(&self) -> crate::Result<()> {
        self.clear_to_default(true).await
    }

    /// Clear the activity, but if there is a running profile, set the activity to that instead
    pub async fn clear_to_default(
        &self,
        reconnect_if_fail: bool,
    ) -> crate::Result<()> {
        let state = State::get().await?;

        let settings = crate::state::Settings::get(&state.pool).await?;
        if !settings.discord_rpc {
            println!("Discord is disabled, clearing activity");
            return self.clear_activity(true).await;
        }

        let running_instances = state.process_manager.get_all();
        if let Some(existing_child) = running_instances.first() {
            self.set_activity(
                &format!("Jugando a {}", existing_child.instance_name),
                None,
                reconnect_if_fail,
            )
            .await?;
        } else {
            self.set_activity("En Strata", Some("Explorando"), reconnect_if_fail)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DiscordDetail;

    #[test]
    fn detail_levels_roundtrip_and_reject_unknown_values() {
        for level in [DiscordDetail::Full, DiscordDetail::Basic] {
            assert_eq!(DiscordDetail::parse(level.as_str()), Some(level));
        }
        assert_eq!(DiscordDetail::parse(" basic\n"), Some(DiscordDetail::Basic), "se tolera el salto de línea final");
        assert_eq!(DiscordDetail::parse(""), None);
        assert_eq!(DiscordDetail::parse("todo"), None);
    }
}
