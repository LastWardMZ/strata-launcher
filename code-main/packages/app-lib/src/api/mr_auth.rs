use crate::state::{FeatureFlag, ModrinthCredentials, Settings};
use serde::Deserialize;

const LOCALHOST_LOGIN_URL: &str = "http://localhost:3000/auth/sign-in";
const LOCALHOST_SIGNUP_URL: &str = "http://localhost:3000/auth/sign-up";

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModrinthAuthFlow {
    SignIn,
    SignUp,
}

#[tracing::instrument]
pub async fn authenticate_begin_flow(
    flow: ModrinthAuthFlow,
) -> crate::Result<&'static str> {
    let state = crate::State::get().await?;
    let settings = Settings::get(&state.pool).await?;
    let use_localhost = settings
        .feature_flags
        .get(&FeatureFlag::LocalhostSignIn)
        .copied()
        .unwrap_or(false);

    Ok(match (use_localhost, flow) {
        (true, ModrinthAuthFlow::SignIn) => LOCALHOST_LOGIN_URL,
        (true, ModrinthAuthFlow::SignUp) => LOCALHOST_SIGNUP_URL,
        (false, ModrinthAuthFlow::SignIn) => crate::state::get_login_url(),
        (false, ModrinthAuthFlow::SignUp) => crate::state::get_signup_url(),
    })
}

#[tracing::instrument]
pub async fn authenticate_finish_flow(
    code: &str,
) -> crate::Result<ModrinthCredentials> {
    let state = crate::State::get().await?;

    let creds = crate::state::finish_login_flow(
        code,
        &state.api_semaphore,
        &state.pool,
    )
    .await?;

    creds.upsert(&state.pool).await?;

    if let Err(error) =
        crate::onboarding_checklist::mark_logged_into_modrinth().await
    {
        tracing::warn!(
            "Failed to mark Modrinth login in onboarding checklist: {error}"
        );
    }

    reconnect_friends(&state).await?;

    Ok(creds)
}

/// Conecta una cuenta de Modrinth con un token personal. Se comprueba que el token es válido y
/// que permite leer la lista de amigos; si no, no se guarda nada.
#[tracing::instrument(skip(token))]
pub async fn authenticate_with_token(
    token: &str,
) -> crate::Result<ModrinthCredentials> {
    let state = crate::State::get().await?;
    let previous = ModrinthCredentials::get_active(&state.pool).await?;

    let creds = crate::state::login_with_token(
        token,
        &state.api_semaphore,
        &state.pool,
    )
    .await?;
    creds.upsert(&state.pool).await?;

    if let Err(error) = crate::api::friends::friends().await {
        // Deshace la conexión: sin permiso de lectura de usuario los amigos no funcionarían.
        ModrinthCredentials::remove(&creds.user_id, &state.pool).await?;
        if let Some(previous) = previous {
            previous.upsert(&state.pool).await?;
        }
        return Err(crate::ErrorKind::InputError(format!(
            "El token es válido pero no permite usar los amigos. Créalo con los permisos \"Leer datos de usuario\" y \"Escribir datos de usuario\" ({error})"
        ))
        .as_error());
    }

    reconnect_friends(&state).await?;

    Ok(creds)
}

#[tracing::instrument]
pub async fn logout() -> crate::Result<()> {
    let state = crate::State::get().await?;
    ModrinthCredentials::deactivate_all(&state.pool).await?;
    state.friends_socket.disconnect().await?;

    Ok(())
}

#[tracing::instrument]
pub async fn get_all() -> crate::Result<Vec<ModrinthCredentials>> {
    let state = crate::State::get().await?;
    ModrinthCredentials::get_all(&state.pool).await
}

#[tracing::instrument]
pub async fn set_active(user_id: &str) -> crate::Result<()> {
    let state = crate::State::get().await?;
    let users = ModrinthCredentials::get_all(&state.pool).await?;
    let Some(mut creds) =
        users.into_iter().find(|creds| creds.user_id == user_id)
    else {
        return Err(crate::ErrorKind::OtherError(format!(
            "Tried to activate nonexistent Modrinth user with ID {user_id}"
        ))
        .as_error());
    };

    creds.active = true;
    creds.upsert(&state.pool).await?;
    reconnect_friends(&state).await?;

    Ok(())
}

#[tracing::instrument]
pub async fn remove_user(user_id: &str) -> crate::Result<()> {
    let state = crate::State::get().await?;
    let current = ModrinthCredentials::get_active(&state.pool).await?;
    ModrinthCredentials::remove(user_id, &state.pool).await?;

    if current.is_some_and(|creds| creds.user_id == user_id) {
        state.friends_socket.disconnect().await?;
    }

    Ok(())
}

async fn reconnect_friends(state: &crate::State) -> crate::Result<()> {
    if let Err(error) = state.friends_socket.disconnect().await {
        tracing::warn!("Failed to disconnect friends socket: {error}");
    }
    if let Err(error) = state
        .friends_socket
        .connect(&state.pool, &state.api_semaphore, &state.process_manager)
        .await
    {
        tracing::warn!("Failed to reconnect friends socket: {error}");
    }

    Ok(())
}

#[tracing::instrument]
pub async fn get_credentials() -> crate::Result<Option<ModrinthCredentials>> {
    let state = crate::State::get().await?;
    let current =
        ModrinthCredentials::get_and_refresh(&state.pool, &state.api_semaphore)
            .await?;
    if current.is_none() {
        state.friends_socket.disconnect().await?;
    }

    Ok(current)
}
