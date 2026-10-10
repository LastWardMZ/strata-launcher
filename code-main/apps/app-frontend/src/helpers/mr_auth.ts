/**
 * All theseus API calls return serialized values (both return values and errors);
 * So, for example, addDefaultInstance creates a blank instance object, where the Rust struct is serialized,
 *  and deserialized into a usable JS object.
 */
import { invoke } from '@tauri-apps/api/core'

export type ModrinthCredentials = {
	session: string
	expires: string
	user_id: string
	active: boolean
}

export type ModrinthAuthFlow = 'sign-in' | 'sign-up'

export async function login(
	flow: ModrinthAuthFlow = 'sign-in',
	addAccount = false,
): Promise<ModrinthCredentials> {
	return await invoke('plugin:mr-auth|modrinth_login', { flow, addAccount })
}

/** Conecta la cuenta de Modrinth con un token personal que el usuario ha creado en su cuenta. */
export async function loginWithToken(token: string): Promise<ModrinthCredentials> {
	return await invoke('plugin:mr-auth|modrinth_login_with_token', { token })
}

/** Evento de ventana que avisa a la app de que la cuenta de Modrinth se ha conectado o desconectado. */
export const MODRINTH_AUTH_CHANGED = 'strata:modrinth-auth-changed'

export function notifyModrinthAuthChanged(): void {
	window.dispatchEvent(new CustomEvent(MODRINTH_AUTH_CHANGED))
}

export async function logout(): Promise<void> {
	return await invoke('plugin:mr-auth|logout')
}

export async function get(): Promise<ModrinthCredentials | null> {
	return await invoke('plugin:mr-auth|get')
}

export async function getAll(): Promise<ModrinthCredentials[]> {
	return await invoke('plugin:mr-auth|get_all')
}

export async function setActive(userId: string): Promise<void> {
	return await invoke('plugin:mr-auth|set_active', { userId })
}

export async function removeUser(userId: string): Promise<void> {
	return await invoke('plugin:mr-auth|remove_account', { userId })
}

export async function cancelLogin(): Promise<void> {
	return await invoke('plugin:mr-auth|cancel_modrinth_login')
}
