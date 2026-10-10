<script setup lang="ts">
import {
	BanIcon,
	CheckIcon,
	CopyIcon,
	PlayIcon,
	SendIcon,
	ServerStackIcon,
	TrashIcon,
	UserPlusIcon,
	UsersIcon,
	XIcon,
} from '@modrinth/assets'
import { Admonition, Button, Combobox, EmptyState, Input } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'

import { useStrataAccount } from '@/composables/use-strata-account'

type FriendStatus = 'outgoing' | 'incoming' | 'accepted' | 'blocked'
type Friend = {
	id: string
	name: string
	status: FriendStatus
	synced: boolean
	via_account?: boolean
}
type InviteInfo = {
	share_id: string
	kind: 'world' | 'server'
	title: string
	game_version: string
}
type ChatMessage = {
	id: string
	from_me: boolean
	text: string
	ts: number
	delivered: boolean
	invite: InviteInfo | null
}
type Share = { id: string; kind: string; title: string; game_version: string; port: number }
type LocalServer = { id: string; name: string; game_version: string; port: number | null }
type Instance = { id: string; name: string; game_version: string }

const call = <T,>(cmd: string, args: Record<string, unknown> = {}) => invoke<T>(cmd, args)

const me = ref<{ id: string; name: string }>({ id: '', name: '' })
const account = useStrataAccount()
const friends = ref<Friend[]>([])
const online = ref<Record<string, boolean>>({})
const selectedId = ref('')
const messages = ref<ChatMessage[]>([])
const draft = ref('')
const addCode = ref('')
const error = ref('')
const notice = ref('')
const shares = ref<Share[]>([])
const servers = ref<LocalServer[]>([])
const lanPort = ref('')
const lanTitle = ref('Mi mundo')
const lanVersion = ref('')
const instances = ref<Instance[]>([])
const instanceId = ref('')
const joined = ref<Record<string, number>>({})
const chatScroll = ref<HTMLElement | null>(null)

const instanceOptions = computed(() =>
	instances.value.map((i) => ({ value: i.id, label: `${i.name} · ${i.game_version}` })),
)
const selectedInstance = computed(
	() => instances.value.find((i) => i.id === instanceId.value) ?? null,
)
const selected = computed(() => friends.value.find((f) => f.id === selectedId.value) ?? null)
const incoming = computed(() => friends.value.filter((f) => f.status === 'incoming'))
const accepted = computed(() => friends.value.filter((f) => f.status === 'accepted'))
const outgoing = computed(() => friends.value.filter((f) => f.status === 'outgoing'))
const blocked = computed(() => friends.value.filter((f) => f.status === 'blocked'))

function fail(e: unknown) {
	error.value = String(e)
	notice.value = ''
}
function say(text: string) {
	notice.value = text
	error.value = ''
}
function time(ts: number) {
	return new Date(ts * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}
function initial(name: string) {
	return (name.trim()[0] ?? '?').toUpperCase()
}

async function loadFriends() {
	friends.value = await call<Friend[]>('p2p_friends')
}
async function loadShares() {
	shares.value = await call<Share[]>('p2p_shares')
}
async function loadMessages() {
	if (!selectedId.value) return
	messages.value = await call<ChatMessage[]>('p2p_messages', { id: selectedId.value })
	await nextTick()
	if (chatScroll.value) chatScroll.value.scrollTop = chatScroll.value.scrollHeight
}
async function refreshOnline() {
	for (const friend of accepted.value) {
		call<boolean>('p2p_is_online', { id: friend.id })
			.then((value) => (online.value = { ...online.value, [friend.id]: value }))
			.catch(() => (online.value = { ...online.value, [friend.id]: false }))
	}
}

async function select(friend: Friend) {
	selectedId.value = friend.id
	messages.value = []
	await loadMessages().catch(fail)
}
async function copyCode() {
	try {
		await navigator.clipboard.writeText(me.value.id)
		say('Código copiado: pásaselo a tu amigo')
	} catch {
		say(me.value.id)
	}
}
async function addFriend() {
	try {
		await call('p2p_add_friend', { code: addCode.value })
		addCode.value = ''
		await loadFriends()
		say('Solicitud enviada')
	} catch (e) {
		fail(e)
	}
}
async function accept(friend: Friend) {
	try {
		await call('p2p_accept_friend', { id: friend.id })
		await loadFriends()
	} catch (e) {
		fail(e)
	}
}
async function remove(friend: Friend) {
	try {
		await call('p2p_remove_friend', { id: friend.id })
		if (selectedId.value === friend.id) selectedId.value = ''
		await loadFriends()
	} catch (e) {
		fail(e)
	}
}
async function block(friend: Friend, value: boolean) {
	try {
		await call('p2p_block_friend', { id: friend.id, blocked: value })
		if (value && selectedId.value === friend.id) selectedId.value = ''
		await loadFriends()
	} catch (e) {
		fail(e)
	}
}
async function send() {
	if (!selected.value || !draft.value.trim()) return
	const text = draft.value
	draft.value = ''
	try {
		await call('p2p_send_message', { id: selected.value.id, text })
		await loadMessages()
	} catch (e) {
		draft.value = text
		fail(e)
	}
}
async function shareServer(server: LocalServer) {
	if (!server.port) return fail('Ese servidor no tiene puerto configurado')
	try {
		await call('p2p_share_start', {
			kind: 'server',
			title: server.name,
			gameVersion: server.game_version,
			port: server.port,
		})
		await loadShares()
		say(`"${server.name}" ya se puede compartir con tus amigos`)
	} catch (e) {
		fail(e)
	}
}
/** Mundo abierto a LAN detectado automáticamente en alguna instancia. */
const lanDetected = ref<{ instanceId: string; port: number } | null>(null)
/** `true` mientras el puerto del formulario lo haya rellenado la detección (y no el usuario). */
let lanAutoFilled = false
const lanDetectedInstance = computed(
	() => instances.value.find((i) => i.id === lanDetected.value?.instanceId) ?? null,
)
const lanShared = computed(() => shares.value.some((s) => s.port === Number(lanPort.value)))

async function detectLanWorlds() {
	if (!instances.value.length) return
	try {
		const found = await call<{ instance_id: string; port: number }[]>('p2p_detect_lan_ports', {
			instanceIds: instances.value.map((i) => i.id),
		})
		const world = found[0]
		if (!world) {
			lanDetected.value = null
			if (lanAutoFilled) {
				lanPort.value = ''
				lanAutoFilled = false
			}
			return
		}
		const isNew =
			lanDetected.value?.port !== world.port || lanDetected.value.instanceId !== world.instance_id
		lanDetected.value = { instanceId: world.instance_id, port: world.port }
		if (!isNew) return
		const instance = instances.value.find((i) => i.id === world.instance_id)
		instanceId.value = world.instance_id
		lanPort.value = String(world.port)
		lanAutoFilled = true
		if (instance) lanVersion.value = instance.game_version
	} catch {
		// Sin acceso a las instancias: se sigue pudiendo escribir el puerto a mano.
	}
}
function markLanManual() {
	lanAutoFilled = false
}
async function shareWorld() {
	const port = Number(lanPort.value)
	if (!Number.isInteger(port) || port < 1 || port > 65535)
		return fail('Abre tu mundo a LAN en Minecraft o escribe su puerto')
	try {
		await call('p2p_share_start', {
			kind: 'world',
			title: lanTitle.value || 'Mi mundo',
			gameVersion: lanVersion.value,
			port,
		})
		await loadShares()
		say('Mundo listo para invitar')
	} catch (e) {
		fail(e)
	}
}
async function stopShare(share: Share) {
	await call('p2p_share_stop', { id: share.id }).catch(fail)
	await loadShares()
}
async function invite(share: Share) {
	if (!selected.value) return fail('Elige primero un amigo en la lista')
	try {
		await call('p2p_invite', { friend: selected.value.id, shareId: share.id })
		await loadMessages()
		say(`Invitación enviada a ${selected.value.name}`)
	} catch (e) {
		fail(e)
	}
}
async function join(friendId: string, info: InviteInfo) {
	try {
		const port = await call<number>('p2p_join', { friend: friendId, shareId: info.share_id })
		joined.value = { ...joined.value, [info.share_id]: port }
		const instance = selectedInstance.value
		if (!instance)
			return say(
				`Conectado. En Minecraft añade el servidor 127.0.0.1:${port}, o elige una instancia para abrirlo directo.`,
			)
		if (info.game_version && instance.game_version !== info.game_version) {
			return say(
				`Conectado, pero tu instancia es ${instance.game_version} y la partida es ${info.game_version}. Usa una instancia ${info.game_version} y entra en 127.0.0.1:${port}.`,
			)
		}
		await call('plugin:worlds|start_join_server', {
			instanceId: instance.id,
			address: `127.0.0.1:${port}`,
		})
		say(`Abriendo "${instance.name}" y entrando a la partida…`)
	} catch (e) {
		fail(e)
	}
}
async function leave(friendId: string, info: InviteInfo) {
	await call('p2p_join_stop', { friend: friendId, shareId: info.share_id }).catch(fail)
	joined.value = Object.fromEntries(
		Object.entries(joined.value).filter(([shareId]) => shareId !== info.share_id),
	)
}

let unlisten: (() => void) | undefined
let timer: ReturnType<typeof setInterval> | undefined
let lanTimer: ReturnType<typeof setInterval> | undefined
onMounted(async () => {
	void call('accounts_sync')
		.then(() => loadFriends())
		.catch(() => {})
	try {
		me.value = await call('p2p_info')
		await Promise.all([loadFriends(), loadShares()])
		servers.value = await call<LocalServer[]>('local_servers_list')
		instances.value = await call<Instance[]>('plugin:instance|instance_list').catch(() => [])
		if (instances.value.length) instanceId.value = instances.value[0].id
		refreshOnline()
		void detectLanWorlds()
	} catch (e) {
		fail(e)
	}
	unlisten = await listen<{ type: string; friend?: string }>('p2p-event', async (event) => {
		await loadFriends().catch(() => {})
		if (event.payload.type === 'message' && event.payload.friend === selectedId.value)
			await loadMessages().catch(() => {})
	})
	timer = setInterval(refreshOnline, 20000)
	lanTimer = setInterval(detectLanWorlds, 4000)
})
onUnmounted(() => {
	unlisten?.()
	if (timer) clearInterval(timer)
	if (lanTimer) clearInterval(lanTimer)
})
</script>

<template>
	<main class="flex flex-col gap-4 p-6">
		<header>
			<h1 class="m-0 flex items-center gap-2 text-2xl font-bold text-contrast">
				<UsersIcon class="size-6" /> Amigos
			</h1>
			<p class="m-0 mt-1 text-secondary">
				El chat y las partidas van directos entre launchers: el servidor de cuentas solo guarda tu
				nombre y tus amistades. Solo tus amigos aceptados pueden escribirte o unirse a lo que
				compartas.
			</p>
		</header>

		<Admonition v-if="error" type="critical" :body="error" dismissible @dismiss="error = ''" />
		<Admonition
			v-else-if="notice"
			type="success"
			:body="notice"
			dismissible
			@dismiss="notice = ''"
		/>

		<section
			class="grid gap-4 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5 lg:grid-cols-3"
		>
			<div class="flex flex-col gap-2">
				<span class="font-semibold text-contrast">Tu cuenta</span>
				<div
					v-if="account.profile.value"
					class="flex items-center gap-2 rounded-xl bg-surface-4 p-2 pl-3"
				>
					<span class="min-w-0 flex-1 truncate font-semibold text-contrast">{{
						account.profile.value.username
					}}</span>
					<span class="text-sm text-secondary">Conectada</span>
				</div>
				<p v-else class="m-0 text-sm text-secondary">
					Sin iniciar sesión. Entra en Ajustes → Perfil para que tus amigos te encuentren por
					nombre; mientras tanto puedes usar códigos.
				</p>
			</div>
			<div class="flex flex-col gap-2">
				<label for="mycode" class="font-semibold text-contrast">Tu código de partidas</label>
				<div class="flex gap-2">
					<Input
						id="mycode"
						:model-value="me.id || 'Iniciando…'"
						readonly
						wrapper-class="flex-grow min-w-0"
					/>
					<Button type="outlined" :disabled="!me.id" @click="copyCode"><CopyIcon /> Copiar</Button>
				</div>
			</div>
			<div class="flex flex-col gap-2">
				<label for="addcode" class="font-semibold text-contrast">Añadir por código</label>
				<div class="flex gap-2">
					<Input
						id="addcode"
						v-model="addCode"
						placeholder="Pega aquí su código"
						autocomplete="off"
						wrapper-class="flex-grow min-w-0"
						@keyup.enter="addFriend"
					/>
					<Button type="colored" color="brand" :disabled="!addCode.trim()" @click="addFriend">
						<UserPlusIcon /> Enviar
					</Button>
				</div>
			</div>
		</section>

		<div class="grid min-h-[420px] gap-4 lg:grid-cols-[320px_1fr]">
			<aside
				class="flex flex-col gap-1 rounded-2xl bg-surface-3 p-3 ring-1 ring-inset ring-surface-5"
			>
				<template v-if="incoming.length">
					<h2 class="m-0 px-2 pt-1 text-base font-bold text-contrast">
						Solicitudes · {{ incoming.length }}
					</h2>
					<div
						v-for="f in incoming"
						:key="f.id"
						class="flex items-center gap-2 rounded-xl bg-surface-4 p-2"
					>
						<div
							class="flex size-9 shrink-0 items-center justify-center rounded-full bg-surface-5 font-bold text-contrast"
						>
							{{ initial(f.name) }}
						</div>
						<span class="min-w-0 flex-1 truncate font-semibold text-contrast">{{ f.name }}</span>
						<Button type="colored" color="brand" size="sm" @click="accept(f)"><CheckIcon /></Button>
						<Button type="outlined" size="sm" @click="remove(f)"><XIcon /></Button>
						<Button type="outlined" color="red" size="sm" @click="block(f, true)"
							><BanIcon
						/></Button>
					</div>
				</template>

				<h2 class="m-0 px-2 pt-2 text-base font-bold text-contrast">
					Amigos · {{ accepted.length }}
				</h2>
				<p v-if="!accepted.length" class="m-0 px-2 py-1 text-sm text-secondary">
					Aún no tienes amigos para jugar. Búscalos por nombre en el panel de la derecha («Añadir un
					amigo») o pega un código a mano.
				</p>
				<button
					v-for="f in accepted"
					:key="f.id"
					class="flex w-full cursor-pointer items-center gap-2 rounded-xl border-0 p-2 text-left transition-colors hover:bg-surface-4"
					:class="f.id === selectedId ? 'bg-surface-4' : 'bg-transparent'"
					@click="select(f)"
				>
					<div class="relative">
						<div
							class="flex size-9 items-center justify-center rounded-full bg-surface-5 font-bold text-contrast"
						>
							{{ initial(f.name) }}
						</div>
						<span
							class="absolute -bottom-0.5 -right-0.5 size-3 rounded-full border-2 border-solid border-surface-3"
							:class="online[f.id] ? 'bg-green' : 'bg-surface-5'"
						/>
					</div>
					<div class="min-w-0 flex-1">
						<div class="truncate font-semibold text-contrast">{{ f.name }}</div>
						<div class="text-xs text-secondary">
							{{ online[f.id] ? 'Conectado' : 'Desconectado'
							}}<template v-if="f.via_account"> · Cuenta</template>
						</div>
					</div>
				</button>

				<template v-if="outgoing.length">
					<h2 class="m-0 px-2 pt-2 text-base font-bold text-contrast">
						Pendientes · {{ outgoing.length }}
					</h2>
					<div v-for="f in outgoing" :key="f.id" class="flex items-center gap-2 p-2">
						<span class="min-w-0 flex-1 truncate text-primary">{{ f.name }}</span>
						<Button type="outlined" size="sm" @click="remove(f)">Cancelar</Button>
					</div>
				</template>

				<template v-if="blocked.length">
					<h2 class="m-0 px-2 pt-2 text-base font-bold text-contrast">
						Bloqueados · {{ blocked.length }}
					</h2>
					<div v-for="f in blocked" :key="f.id" class="flex items-center gap-2 p-2">
						<span class="min-w-0 flex-1 truncate text-primary">{{ f.name }}</span>
						<Button type="outlined" size="sm" @click="block(f, false)">Desbloquear</Button>
					</div>
				</template>
			</aside>

			<section
				class="flex min-h-0 flex-col rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<template v-if="selected">
					<div
						class="flex items-center justify-between gap-2 border-0 border-b border-solid border-surface-5 pb-3"
					>
						<div class="flex min-w-0 items-center gap-2">
							<div
								class="flex size-9 shrink-0 items-center justify-center rounded-full bg-surface-5 font-bold text-contrast"
							>
								{{ initial(selected.name) }}
							</div>
							<strong class="truncate text-lg text-contrast">{{ selected.name }}</strong>
						</div>
						<div class="flex gap-2">
							<Button type="outlined" size="sm" @click="remove(selected)"
								><TrashIcon /> Eliminar</Button
							>
							<Button type="outlined" color="red" size="sm" @click="block(selected, true)"
								><BanIcon /> Bloquear</Button
							>
						</div>
					</div>
					<div
						ref="chatScroll"
						class="flex max-h-[460px] min-h-[260px] flex-1 flex-col gap-2 overflow-y-auto py-3"
					>
						<p v-if="!messages.length" class="m-auto text-center text-secondary">
							Escribe el primer mensaje. Si no está conectado, se entregará cuando vuelva.
						</p>
						<div
							v-for="m in messages"
							:key="m.id"
							class="flex max-w-[72%] flex-col gap-1 break-words rounded-2xl px-3 py-2"
							:class="
								m.from_me
									? 'self-end bg-brand-highlight text-contrast'
									: 'self-start bg-surface-4 text-primary'
							"
						>
							<template v-if="m.invite">
								<strong class="flex items-center gap-1.5 text-contrast">
									<component
										:is="m.invite.kind === 'world' ? PlayIcon : ServerStackIcon"
										class="size-4"
									/>
									{{ m.from_me ? 'Invitaste' : 'Te invita' }} a
									{{ m.invite.kind === 'world' ? 'un mundo' : 'un servidor' }}
								</strong>
								<span>
									{{ m.invite.title
									}}<template v-if="m.invite.game_version"> · {{ m.invite.game_version }}</template>
								</span>
								<template v-if="!m.from_me">
									<div v-if="joined[m.invite.share_id]" class="flex flex-wrap items-center gap-2">
										<span class="text-sm"
											>Conectado en <code>127.0.0.1:{{ joined[m.invite.share_id] }}</code></span
										>
										<Button type="outlined" size="sm" @click="leave(selected.id, m.invite)"
											>Desconectar</Button
										>
									</div>
									<div v-else>
										<Button
											type="colored"
											color="brand"
											size="sm"
											@click="join(selected.id, m.invite)"
										>
											<PlayIcon /> Unirme
										</Button>
									</div>
								</template>
							</template>
							<template v-else>{{ m.text }}</template>
							<small class="text-xs text-secondary">
								{{ time(m.ts)
								}}<template v-if="m.from_me">
									· {{ m.delivered ? 'entregado' : 'pendiente' }}</template
								>
							</small>
						</div>
					</div>
					<form class="flex gap-2" @submit.prevent="send">
						<Input
							v-model="draft"
							:maxlength="2000"
							placeholder="Escribe un mensaje…"
							autocomplete="off"
							wrapper-class="flex-grow"
						/>
						<Button type="colored" color="brand" native-type="submit" :disabled="!draft.trim()"
							><SendIcon /> Enviar</Button
						>
					</form>
				</template>
				<EmptyState
					v-else
					heading="Elige un amigo"
					description="Selecciona a alguien de la lista para chatear o invitarle a tu partida."
				/>
			</section>
		</div>

		<section
			class="flex flex-col gap-3 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
		>
			<div class="flex flex-wrap items-start justify-between gap-4">
				<div class="min-w-0 flex-1">
					<h2 class="m-0 text-lg font-bold text-contrast">Compartir con amigos</h2>
					<p class="m-0 mt-1 text-secondary">
						Lo compartido solo es accesible para tus amigos aceptados, y deja de serlo al dejar de
						compartirlo o al cerrar Strata.
					</p>
				</div>
				<div class="flex w-72 max-w-full flex-col gap-1.5">
					<span class="font-semibold text-contrast">Tu instancia de Minecraft</span>
					<Combobox
						v-model="instanceId"
						:options="instanceOptions"
						placeholder="No hay instancias"
						:disabled="!instances.length"
					/>
					<span class="text-xs text-secondary">
						Para detectar tu mundo LAN y abrir el juego al aceptar una invitación.
					</span>
				</div>
			</div>

			<div class="grid gap-4 lg:grid-cols-2">
				<div class="flex flex-col gap-2 rounded-xl bg-surface-4 p-3">
					<h3 class="m-0 font-bold text-contrast">Mundo en LAN</h3>
					<p
						v-if="lanDetected"
						class="m-0 flex items-center gap-2 text-sm font-semibold text-contrast"
					>
						<span class="size-2.5 shrink-0 rounded-full bg-green" />
						Mundo detectado en «{{ lanDetectedInstance?.name ?? 'tu instancia' }}» · puerto
						{{ lanDetected.port }}
					</p>
					<p v-else class="m-0 flex items-center gap-2 text-sm text-secondary">
						<span class="size-2.5 shrink-0 animate-pulse rounded-full bg-surface-5" />
						Esperando un mundo abierto a LAN. En Minecraft, pulsa Esc y «Abrir a LAN»; se detecta
						solo.
					</p>
					<div class="flex flex-wrap gap-2">
						<Input
							v-model="lanTitle"
							placeholder="Nombre"
							:maxlength="32"
							wrapper-class="flex-grow min-w-[8rem]"
						/>
						<Input
							v-model="lanVersion"
							placeholder="Versión (1.21.1)"
							:maxlength="32"
							wrapper-class="w-36"
						/>
						<Input
							v-model="lanPort"
							placeholder="Puerto"
							inputmode="numeric"
							wrapper-class="w-24"
							@update:model-value="markLanManual"
							@keyup.enter="shareWorld"
						/>
					</div>
					<div class="flex gap-2">
						<Button
							type="colored"
							color="brand"
							:disabled="!lanPort || lanShared"
							@click="shareWorld"
						>
							<CheckIcon v-if="lanShared" /><PlayIcon v-else />
							{{ lanShared ? 'Compartido' : 'Compartir' }}
						</Button>
					</div>
				</div>
				<div class="flex flex-col gap-2 rounded-xl bg-surface-4 p-3">
					<h3 class="m-0 font-bold text-contrast">Servidores personales</h3>
					<p v-if="!servers.length" class="m-0 text-sm text-secondary">
						No tienes servidores personales.
					</p>
					<div v-for="s in servers" :key="s.id" class="flex items-center gap-2">
						<ServerStackIcon class="size-5 shrink-0 text-secondary" />
						<span class="min-w-0 flex-1 truncate text-primary">
							{{ s.name }} <span class="text-secondary">:{{ s.port ?? '—' }}</span>
						</span>
						<Button type="outlined" size="sm" :disabled="!s.port" @click="shareServer(s)"
							>Compartir</Button
						>
					</div>
				</div>
			</div>

			<div class="flex flex-col gap-2">
				<h3 class="m-0 font-bold text-contrast">Compartiendo ahora</h3>
				<p v-if="!shares.length" class="m-0 text-sm text-secondary">Nada por ahora.</p>
				<div
					v-for="s in shares"
					:key="s.id"
					class="flex flex-wrap items-center gap-2 rounded-xl bg-surface-4 p-2 pl-3"
				>
					<component
						:is="s.kind === 'world' ? PlayIcon : ServerStackIcon"
						class="size-5 shrink-0 text-secondary"
					/>
					<span class="min-w-0 flex-1 truncate text-primary">
						{{ s.kind === 'world' ? 'Mundo' : 'Servidor' }}: {{ s.title }}
						<span class="text-secondary">:{{ s.port }}</span>
					</span>
					<Button type="colored" color="brand" size="sm" :disabled="!selected" @click="invite(s)">
						<SendIcon /> Invitar a {{ selected?.name ?? 'amigo' }}
					</Button>
					<Button type="outlined" size="sm" @click="stopShare(s)">Dejar de compartir</Button>
				</div>
			</div>
		</section>
	</main>
</template>
