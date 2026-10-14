<script setup lang="ts">
/**
 * Panel de amigos sobre el juego (Shift+Tab), como el de Steam: lista de amigos agrupada por estado, chat con cada uno,
 * «Invitar a jugar» y «Unirme» en las invitaciones. Vive en su propia ventana, que el launcher coloca encima de Minecraft.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

import UserAvatar from '@/components/ui/friends/UserAvatar.vue'
import { inviteToPlay } from '@/composables/use-invite-to-play'
import {
	findMatchingInstance,
	installRecipe,
	type InstanceRecipe,
	joinInvite,
	type LocalInstance,
	recipeSummary,
} from '@/composables/use-instance-sharing'

type Friend = { id: string; name: string; status: 'outgoing' | 'incoming' | 'accepted' | 'blocked' }
type Invite = { share_id: string; kind: 'world' | 'server' | 'instance'; title: string; game_version: string; instance?: InstanceRecipe | null }
type Message = { id: string; from_me: boolean; text: string; ts: number; delivered: boolean; invite: Invite | null }
type Presence = { online: boolean; playing: { title: string; since: number } | null; busy: boolean }
type Pending = { friend: Friend; message: Message; invite: Invite }

/** Una invitación sin responder deja de ofrecerse pasado este tiempo (los mundos LAN no duran). */
const INVITE_FRESH_SECONDS = 20 * 60

const friends = ref<Friend[]>([])
const presence = ref<Record<string, Presence>>({})
const instances = ref<LocalInstance[]>([])
const pending = ref<Pending[]>([])
const selectedId = ref('')
const messages = ref<Message[]>([])
const draft = ref('')
const notice = ref('')
const error = ref('')
const busy = ref(false)
const scroller = ref<HTMLElement | null>(null)
const draftInput = ref<HTMLInputElement | null>(null)

const accepted = computed(() => friends.value.filter((friend) => friend.status === 'accepted'))
const selected = computed(() => accepted.value.find((friend) => friend.id === selectedId.value) ?? null)
const byName = (a: Friend, b: Friend) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })
const groups = computed(() => [
	{ key: 'playing', label: 'Jugando', users: accepted.value.filter((f) => presence.value[f.id]?.playing).sort(byName) },
	{
		key: 'online',
		label: 'En línea',
		users: accepted.value.filter((f) => presence.value[f.id]?.online && !presence.value[f.id]?.playing).sort(byName),
	},
	{ key: 'offline', label: 'Desconectados', users: accepted.value.filter((f) => !presence.value[f.id]?.online).sort(byName) },
])

function describe(cause: unknown) {
	if (typeof cause === 'string') return cause
	if (cause instanceof Error) return cause.message
	if (cause && typeof cause === 'object' && 'message' in cause) return String((cause as { message: unknown }).message)
	return 'Algo ha fallado. Inténtalo de nuevo.'
}
function say(text: string) {
	notice.value = text
	error.value = ''
}
function fail(cause: unknown) {
	error.value = describe(cause)
	notice.value = ''
}
function time(ts: number) {
	return new Date(ts * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}
function statusOf(friend: Friend): 'online' | 'offline' | 'playing' | 'busy' {
	const state = presence.value[friend.id]
	if (!state?.online) return 'offline'
	if (state.busy) return 'busy'
	return state.playing ? 'playing' : 'online'
}
function subtitle(friend: Friend) {
	const state = presence.value[friend.id]
	if (!state?.online) return 'Desconectado'
	if (state.playing) return `Jugando a ${state.playing.title}`
	return state.busy ? 'No molestar' : 'En línea'
}

/** Cierra el panel; con `refocus` el foco vuelve al juego. */
async function hide(refocus = true) {
	await invoke('overlay_hide', { refocus }).catch(() => getCurrentWindow().hide())
}

async function loadFriends() {
	friends.value = await invoke<Friend[]>('p2p_friends')
	const entries = await Promise.all(
		accepted.value.map(async (friend) => {
			try {
				return [friend.id, await invoke<Presence>('p2p_presence', { id: friend.id })] as const
			} catch {
				return [friend.id, { online: false, playing: null, busy: false }] as const
			}
		}),
	)
	presence.value = Object.fromEntries(entries)
}
async function loadPending() {
	const now = Date.now() / 1000
	const found: Pending[] = []
	for (const friend of accepted.value) {
		const list = await invoke<Message[]>('p2p_messages', { id: friend.id }).catch(() => [] as Message[])
		const last = [...list].reverse().find((m) => !m.from_me && m.invite && m.invite.kind !== 'instance')
		if (last?.invite && now - last.ts < INVITE_FRESH_SECONDS) found.push({ friend, message: last, invite: last.invite })
	}
	pending.value = found
}
async function loadMessages() {
	if (!selectedId.value) return
	messages.value = await invoke<Message[]>('p2p_messages', { id: selectedId.value })
	await nextTick()
	if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight
}
async function refreshAll() {
	try {
		await loadFriends()
		instances.value = await invoke<LocalInstance[]>('plugin:instance|instance_list').catch(() => instances.value)
		await Promise.all([loadPending(), loadMessages()])
	} catch (cause) {
		fail(cause)
	}
}

async function open(friend: Friend) {
	selectedId.value = friend.id
	messages.value = []
	notice.value = ''
	error.value = ''
	await loadMessages().catch(fail)
	await nextTick()
	draftInput.value?.focus()
}
function back() {
	selectedId.value = ''
	messages.value = []
	void loadPending()
}
async function send() {
	const text = draft.value.trim()
	if (!selected.value || !text) return
	draft.value = ''
	try {
		await invoke('p2p_send_message', { id: selected.value.id, text })
		await loadMessages()
	} catch (cause) {
		draft.value = text
		fail(cause)
	}
}

async function invitePlay(friend: Friend) {
	if (busy.value) return
	busy.value = true
	try {
		say(await inviteToPlay(friend.id, friend.name))
		await loadMessages()
	} catch (cause) {
		fail(cause)
	} finally {
		busy.value = false
	}
}
const matchFor = (invite: Invite) => findMatchingInstance(instances.value, invite.instance, invite.game_version)
const needsInstall = (invite: Invite) => !!invite.instance && !matchFor(invite)

/** «Unirme»: entra a la partida y devuelve el control al juego (el juego se conecta solo). */
async function join(friend: Friend, invite: Invite) {
	if (busy.value) return
	busy.value = true
	try {
		say(await joinInvite(friend.id, invite, instances.value))
		setTimeout(() => void hide(), 700)
	} catch (cause) {
		fail(cause)
	} finally {
		busy.value = false
	}
}
async function installAndJoin(friend: Friend, invite: Invite) {
	if (busy.value || !invite.instance) return
	busy.value = true
	say(`Instalando «${invite.instance.name}»… Puedes ver el progreso en el launcher.`)
	try {
		await installRecipe(invite.instance)
		instances.value = await invoke<LocalInstance[]>('plugin:instance|instance_list').catch(() => instances.value)
		say(await joinInvite(friend.id, invite, instances.value))
		setTimeout(() => void hide(), 700)
	} catch (cause) {
		fail(cause)
	} finally {
		busy.value = false
	}
}

let unlisten: UnlistenFn[] = []
let focusedOnce = false
let poll: ReturnType<typeof setInterval> | undefined
function onKey(event: KeyboardEvent) {
	// Esc o Shift+Tab (la misma tecla que lo abrió) devuelven el juego.
	if (event.key === 'Escape' || (event.key === 'Tab' && event.shiftKey)) {
		event.preventDefault()
		void hide()
	}
}
onMounted(async () => {
	window.addEventListener('keydown', onKey)
	void refreshAll()
	unlisten.push(
		await listen('overlay-shown', () => {
			focusedOnce = false
			notice.value = ''
			error.value = ''
			void refreshAll()
		}),
		await listen('p2p-event', () => void refreshAll()),
		// Si el usuario pulsa en otra ventana (p. ej. vuelve al juego con el ratón), el panel se retira solo.
		await getCurrentWindow().onFocusChanged(({ payload: focused }) => {
			if (focused) focusedOnce = true
			else if (focusedOnce) void hide(false)
		}),
	)
	poll = setInterval(() => {
		if (!document.hidden) void loadFriends().catch(() => {})
	}, 15000)
})
onBeforeUnmount(() => {
	window.removeEventListener('keydown', onKey)
	unlisten.forEach((off) => off())
	clearInterval(poll)
})
</script>

<template>
	<div class="fixed inset-0 flex justify-end bg-black/60 p-4 text-primary" @mousedown.self="hide()">
		<section
			class="flex h-full w-[380px] max-w-full flex-col overflow-hidden rounded-2xl border border-solid border-surface-5 bg-surface-2 shadow-2xl"
			aria-label="Amigos de Strata"
		>
			<header class="flex items-center gap-2 border-0 border-b border-solid border-surface-5 bg-surface-3 px-3 py-2.5">
				<button
					v-if="selected"
					type="button"
					class="cursor-pointer rounded-lg border-0 bg-transparent px-2 py-1 text-lg text-secondary hover:bg-surface-4 hover:text-contrast"
					aria-label="Volver a la lista de amigos"
					@click="back"
				>
					←
				</button>
				<h1 class="m-0 flex-1 truncate text-base font-bold text-contrast">
					{{ selected ? selected.name : 'Amigos' }}
				</h1>
				<button
					v-if="selected"
					type="button"
					class="cursor-pointer rounded-lg border-0 bg-brand px-3 py-1.5 text-sm font-semibold text-[var(--color-accent-contrast)] disabled:opacity-50"
					:disabled="busy"
					@click="invitePlay(selected)"
				>
					Invitar a jugar
				</button>
				<button
					type="button"
					class="cursor-pointer rounded-lg border-0 bg-transparent px-2 py-1 text-lg text-secondary hover:bg-surface-4 hover:text-contrast"
					aria-label="Cerrar y volver al juego"
					@click="hide()"
				>
					✕
				</button>
			</header>

			<p v-if="error" class="m-0 bg-red/15 px-3 py-2 text-sm text-red" role="alert">{{ error }}</p>
			<p v-else-if="notice" class="m-0 bg-brand/15 px-3 py-2 text-sm text-contrast" role="status">{{ notice }}</p>

			<!-- Lista de amigos -->
			<div v-if="!selected" class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
				<div
					v-for="item in pending"
					:key="item.message.id"
					class="flex flex-col gap-2 rounded-xl border border-solid border-brand bg-brand-highlight p-2.5"
					role="group"
					:aria-label="`${item.friend.name} te ha invitado a jugar`"
				>
					<div class="flex items-center gap-2">
						<UserAvatar :name="item.friend.name" :size="32" />
						<span class="min-w-0 flex-1 text-sm leading-tight text-contrast">
							<strong>{{ item.friend.name }}</strong> te invita a jugar a «{{ item.invite.title }}»
						</span>
					</div>
					<button
						v-if="needsInstall(item.invite)"
						type="button"
						class="cursor-pointer rounded-lg border-0 bg-brand px-3 py-1.5 text-sm font-semibold text-[var(--color-accent-contrast)] disabled:opacity-50"
						:disabled="busy"
						@click="installAndJoin(item.friend, item.invite)"
					>
						Instalar y unirme
					</button>
					<button
						v-else
						type="button"
						class="cursor-pointer rounded-lg border-0 bg-brand px-3 py-1.5 text-sm font-semibold text-[var(--color-accent-contrast)] disabled:opacity-50"
						:disabled="busy"
						@click="join(item.friend, item.invite)"
					>
						Unirme
					</button>
					<p v-if="needsInstall(item.invite) && item.invite.instance" class="m-0 text-xs text-secondary">
						Necesitas «{{ item.invite.instance.name }}» ({{ recipeSummary(item.invite.instance) }}).
					</p>
				</div>

				<p v-if="!accepted.length" class="m-0 text-sm text-secondary">
					Aún no tienes amigos con los que chatear. Añádelos desde el launcher.
				</p>
				<template v-for="group in groups" :key="group.key">
					<div v-if="group.users.length" class="flex flex-col gap-0.5">
						<div class="px-1 pb-1 text-xs font-semibold uppercase tracking-wide text-secondary">
							{{ group.label }} · {{ group.users.length }}
						</div>
						<div
							v-for="friend in group.users"
							:key="friend.id"
							class="group flex items-center gap-1 rounded-xl p-1.5 hover:bg-surface-3 focus-within:bg-surface-3"
						>
							<button
								type="button"
								class="flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 rounded-lg border-0 bg-transparent p-0 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
								:aria-label="`Abrir chat con ${friend.name}`"
								@click="open(friend)"
							>
								<UserAvatar :name="friend.name" :size="36" :status="statusOf(friend)" />
								<span class="min-w-0 flex-1">
									<span
										class="block truncate font-semibold leading-tight"
										:class="group.key === 'offline' ? 'text-secondary' : 'text-contrast'"
										>{{ friend.name }}</span
									>
									<span
										class="block truncate text-xs leading-tight"
										:class="group.key === 'playing' ? 'text-brand' : 'text-secondary'"
										>{{ subtitle(friend) }}</span
									>
								</span>
							</button>
							<button
								v-if="group.key !== 'offline'"
								type="button"
								class="shrink-0 cursor-pointer rounded-lg border-0 bg-surface-4 px-2.5 py-1 text-xs font-semibold text-contrast opacity-0 transition-opacity hover:bg-surface-5 focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 disabled:opacity-40"
								:disabled="busy"
								:aria-label="`Invitar a jugar a ${friend.name}`"
								@click="invitePlay(friend)"
							>
								Invitar
							</button>
						</div>
					</div>
				</template>
			</div>

			<!-- Chat -->
			<template v-else>
				<div ref="scroller" class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3">
					<p v-if="!messages.length" class="m-0 text-sm text-secondary">Aún no habéis hablado. Escribe el primer mensaje.</p>
					<div
						v-for="message in messages"
						:key="message.id"
						class="flex flex-col gap-1"
						:class="message.from_me ? 'items-end' : 'items-start'"
					>
						<div
							class="max-w-[85%] rounded-2xl px-3 py-1.5 text-sm"
							:class="message.from_me ? 'bg-brand text-[var(--color-accent-contrast)]' : 'bg-surface-4 text-contrast'"
						>
							<template v-if="message.invite">
								<strong class="block">{{
									message.from_me ? 'Has invitado a jugar' : `${selected.name} te invita a jugar`
								}}</strong>
								<span class="block opacity-90">{{ message.invite.title }}</span>
								<button
									v-if="!message.from_me && message.invite.kind !== 'instance'"
									type="button"
									class="mt-1.5 cursor-pointer rounded-lg border-0 bg-surface-1 px-3 py-1 text-xs font-semibold text-contrast disabled:opacity-50"
									:disabled="busy"
									@click="needsInstall(message.invite) ? installAndJoin(selected, message.invite) : join(selected, message.invite)"
								>
									{{ needsInstall(message.invite) ? 'Instalar y unirme' : 'Unirme' }}
								</button>
							</template>
							<template v-else>{{ message.text }}</template>
						</div>
						<span class="px-1 text-[11px] text-secondary">{{ time(message.ts) }}{{ message.from_me && !message.delivered ? ' · sin entregar' : '' }}</span>
					</div>
				</div>
				<form class="flex gap-2 border-0 border-t border-solid border-surface-5 p-2.5" @submit.prevent="send">
					<input
						ref="draftInput"
						v-model="draft"
						type="text"
						maxlength="500"
						autocomplete="off"
						:placeholder="`Escribe a ${selected.name}…`"
						:aria-label="`Mensaje para ${selected.name}`"
						class="min-w-0 flex-1 rounded-xl border border-solid border-surface-5 bg-surface-3 px-3 py-2 text-sm text-contrast outline-none focus:border-brand"
					/>
					<button
						type="submit"
						class="cursor-pointer rounded-xl border-0 bg-brand px-3.5 py-2 text-sm font-semibold text-[var(--color-accent-contrast)] disabled:opacity-50"
						:disabled="!draft.trim()"
					>
						Enviar
					</button>
				</form>
			</template>

			<footer class="border-0 border-t border-solid border-surface-5 px-3 py-1.5 text-center text-xs text-secondary">
				Shift+Tab o Esc para volver al juego
			</footer>
		</section>
	</div>
</template>
