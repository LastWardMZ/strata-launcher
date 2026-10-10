<script setup lang="ts">
import { CheckIcon, ExternalIcon, LogInIcon, LogOutIcon } from '@modrinth/assets'
import { Admonition, Avatar, Button, injectModrinthClient, Input } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { onBeforeUnmount, onMounted, ref } from 'vue'

import { get_user } from '@/helpers/cache'
import { syncModrinthFriendsToP2p } from '@/helpers/modrinth-p2p-sync'
import {
	get as getCredentials,
	loginWithToken,
	logout,
	MODRINTH_AUTH_CHANGED,
	notifyModrinthAuthChanged,
} from '@/helpers/mr_auth'
import { BIO_MAX_LENGTH, bioWithMarker, bioWithoutMarker, codeFromBio } from '@/helpers/p2p-link'

type ConnectedUser = {
	id: string
	username: string
	avatar_url?: string | null
	bio?: string | null
}

const client = injectModrinthClient()

const loading = ref(true)
const user = ref<ConnectedUser | null>(null)
const token = ref('')
const busy = ref(false)
const error = ref('')
const myCode = ref('')
const published = ref(false)
const publishBusy = ref(false)

const TOKENS_URL = 'https://modrinth.com/settings/pats'

async function load() {
	try {
		const credentials = await getCredentials()
		user.value = credentials?.user_id
			? ((await get_user(credentials.user_id, 'bypass').catch(() => null)) ?? {
					id: credentials.user_id,
					username: credentials.user_id,
				})
			: null
		await refreshPublished()
	} finally {
		loading.value = false
	}
}

async function refreshPublished() {
	try {
		myCode.value = (await invoke<{ id: string }>('p2p_info')).id
		published.value = !!user.value && codeFromBio(user.value.bio) === myCode.value
	} catch {
		published.value = false
	}
}

/** Publica (o retira) el código de partidas en la biografía de Modrinth para enlazar con los amigos. */
async function setPublished(enable: boolean) {
	if (!user.value || !myCode.value || publishBusy.value) return
	publishBusy.value = true
	error.value = ''
	try {
		const next = enable
			? bioWithMarker(user.value.bio, myCode.value)
			: bioWithoutMarker(user.value.bio)
		if (next === null) {
			error.value = `Tu biografía de Modrinth es demasiado larga: el código ocupa 50 de los ${BIO_MAX_LENGTH} caracteres que admite. Acórtala en Modrinth y vuelve a intentarlo.`
			return
		}
		await client.labrinth.users_v2.patch(user.value.id, { bio: next })
		await load()
		void syncModrinthFriendsToP2p()
	} catch (cause) {
		error.value =
			typeof cause === 'string'
				? cause
				: ((cause as { message?: string })?.message ?? 'No se pudo actualizar tu biografía.')
	} finally {
		publishBusy.value = false
	}
}

async function connect() {
	if (!token.value.trim() || busy.value) return
	busy.value = true
	error.value = ''
	try {
		await loginWithToken(token.value)
		token.value = ''
		await load()
		notifyModrinthAuthChanged()
	} catch (cause) {
		const text =
			typeof cause === 'string'
				? cause
				: ((cause as { message?: string })?.message ?? 'No se pudo conectar la cuenta.')
		// El error de Rust antepone «Invalid input: » a los mensajes de validación.
		error.value = text.replace(/^Invalid input:\s*/i, '')
	} finally {
		busy.value = false
	}
}

async function disconnect() {
	if (busy.value) return
	busy.value = true
	error.value = ''
	try {
		await logout()
		user.value = null
		notifyModrinthAuthChanged()
	} catch (cause) {
		error.value = (cause as { message?: string })?.message ?? 'No se pudo desconectar la cuenta.'
	} finally {
		busy.value = false
	}
}

onMounted(() => {
	void load()
	window.addEventListener(MODRINTH_AUTH_CHANGED, load)
})
onBeforeUnmount(() => window.removeEventListener(MODRINTH_AUTH_CHANGED, load))
</script>

<template>
	<section
		class="flex flex-col gap-3 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
	>
		<div>
			<h3 class="m-0 text-lg font-bold text-contrast">Cuenta de Modrinth</h3>
			<p class="m-0 mt-1 text-secondary">
				Strata usa tu cuenta de Modrinth para gestionar tus amigos y tu privacidad. Se conecta con
				un token personal que controlas tú y puedes revocar cuando quieras.
			</p>
		</div>

		<Admonition v-if="error" type="critical" :body="error" dismissible @dismiss="error = ''" />

		<div v-if="loading" class="text-secondary">Comprobando la conexión…</div>

		<div v-else-if="user" class="flex flex-wrap items-center gap-3">
			<Avatar :src="user.avatar_url ?? undefined" size="40px" circle />
			<div class="min-w-0 flex-1">
				<div class="flex items-center gap-1.5 font-semibold text-contrast">
					<CheckIcon class="size-4 text-green" /> Conectado como {{ user.username }}
				</div>
				<div class="text-sm text-secondary">
					Tus amigos y tus opciones de privacidad se gestionan desde esta cuenta.
				</div>
			</div>
			<Button type="outlined" :disabled="busy" @click="disconnect">
				<LogOutIcon /> Desconectar
			</Button>
		</div>

		<div v-if="user" class="flex flex-col gap-2 rounded-xl bg-surface-4 p-3">
			<div class="flex flex-wrap items-center justify-between gap-3">
				<div class="min-w-0 flex-1">
					<strong class="text-contrast">Chat y partidas con tus amigos</strong>
					<p class="m-0 text-sm text-secondary">
						Para que tus amigos que usan Strata te encuentren solos, puedes publicar tu código de
						partidas en tu biografía de Modrinth (añade una línea «strata:…» de 50 caracteres,
						visible en tu perfil público). Así podéis chatear y jugar sin intercambiar códigos.
						Puedes quitarlo cuando quieras.
					</p>
				</div>
				<Button
					v-if="published"
					type="outlined"
					:disabled="publishBusy"
					@click="setPublished(false)"
				>
					<CheckIcon class="text-green" /> Publicado · Quitar
				</Button>
				<Button
					v-else
					type="colored"
					color="brand"
					:disabled="publishBusy || !myCode"
					@click="setPublished(true)"
				>
					Publicar mi código
				</Button>
			</div>
		</div>

		<template v-else>
			<ol class="m-0 flex list-decimal flex-col gap-1.5 pl-5 text-secondary">
				<li>
					Abre la página de tokens de tu cuenta de Modrinth
					<Button type="outlined" size="sm" class="ml-1 inline-flex" @click="openUrl(TOKENS_URL)">
						<ExternalIcon /> Abrir
					</Button>
				</li>
				<li>
					Crea un token nuevo (por ejemplo, llamado «Strata») con los permisos
					<strong class="text-contrast">Leer datos de usuario</strong> y
					<strong class="text-contrast">Escribir datos de usuario</strong>.
				</li>
				<li>Copia el token y pégalo aquí.</li>
			</ol>
			<div class="flex gap-2">
				<Input
					v-model="token"
					type="password"
					autocomplete="off"
					placeholder="mrp_…"
					wrapper-class="flex-grow min-w-0"
					@keyup.enter="connect"
				/>
				<Button type="colored" color="brand" :disabled="busy || !token.trim()" @click="connect">
					<LogInIcon /> Conectar
				</Button>
			</div>
			<p class="m-0 text-sm text-secondary">
				El token se guarda solo en este equipo. Si lo revocas en Modrinth, Strata dejará de poder
				usar tu cuenta.
			</p>
		</template>
	</section>
</template>
