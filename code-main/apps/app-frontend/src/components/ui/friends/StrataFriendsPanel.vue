<script setup lang="ts">
import {
	BanIcon,
	CheckIcon,
	ReportIcon,
	MoreVerticalIcon,
	PlayIcon,
	SendIcon,
	SearchIcon,
	UserPlusIcon,
	UsersIcon,
	XIcon,
} from '@modrinth/assets'
import {
	Button,
	Chips,
	IconButton,
	injectNotificationManager,
	Input,
	TeleportOverflowMenu,
	Textarea,
	Toggle,
} from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import ModalWrapper from '@/components/ui/modal/ModalWrapper.vue'
import UserAvatar from '@/components/ui/friends/UserAvatar.vue'
import { useFriendAvatars } from '@/composables/use-friend-avatars'
import { useFriendPresence } from '@/composables/use-friend-presence'
import { useJoinRequests } from '@/composables/use-join-requests'
import { useProfileAvatar } from '@/composables/use-profile-avatar'
import { type Relation, type SearchHit, useStrataAccount } from '@/composables/use-strata-account'

const emit = defineEmits<{ signIn: []; editProfile: [] }>()
const router = useRouter()
const account = useStrataAccount()
const { loggedIn, social, busy, error, status, profile } = account
const { avatar } = useProfileAvatar()
const { addNotification } = injectNotificationManager()
const nameOf = (id: string) => social.value.friends.find((friend) => friend.id === id)?.username ?? 'Un amigo'
const { requests: joinRequests, shares: openShares, respond } = useJoinRequests({
	onRequest: (friend) => {
		addNotification({ type: 'info', title: 'Quieren unirse a tu partida', text: `${nameOf(friend)} te ha pedido unirse. Respóndele en el panel de amigos.` })
		// Si el launcher está en segundo plano, parpadea en la barra de tareas.
		if (!document.hasFocus()) void getCurrentWindow().requestUserAttention(UserAttentionType.Informational).catch(() => {})
	},
	onDeclined: (friend) =>
		addNotification({ type: 'info', title: 'Petición rechazada', text: `${nameOf(friend)} no puede unirse ahora mismo.` }),
})
// «No molestar»: ajuste local; los amigos ven que estás ocupado y no pueden pedirte unirte.
const doNotDisturb = ref(false)
onMounted(async () => {
	try {
		doNotDisturb.value = await invoke<boolean>('p2p_do_not_disturb')
	} catch {
		// Se queda desactivado.
	}
})
async function setDoNotDisturb(value: boolean) {
	try {
		await invoke('p2p_set_do_not_disturb', { enabled: value })
		doNotDisturb.value = value
	} catch (cause) {
		addNotification({ type: 'error', title: 'No se pudo cambiar «No molestar»', text: typeof cause === 'string' ? cause : 'Inténtalo de nuevo.' })
	}
}

// Denuncia a un amigo: el motivo se guarda en el servicio de cuentas para revisarlo.
const reportModal = ref<InstanceType<typeof ModalWrapper> | null>(null)
const reportTarget = ref('')
const reportReason = ref('')
function openReport(username: string) {
	reportTarget.value = username
	reportReason.value = ''
	reportModal.value?.show()
}
async function sendReport() {
	const reason = reportReason.value.trim()
	if (!reason) return
	if (await account.report(reportTarget.value, reason)) {
		reportModal.value?.hide()
		addNotification({
			type: 'success',
			title: 'Denuncia enviada',
			text: `Gracias. Si quieres dejar de ver a ${reportTarget.value}, también puedes bloquearlo.`,
		})
	}
}

/** Mundo o servidor elegido para cada petición cuando se comparten varios. */
const chosenShare = ref<Record<string, string>>({})
const joinError = ref('')

async function answerJoin(friend: string, accept: boolean) {
	joinError.value = (await respond(friend, accept, accept ? chosenShare.value[friend] : undefined)) ?? ''
}
async function askToJoin(user: { id: string; username: string }) {
	try {
		await invoke('p2p_request_join', { friend: user.id })
		addNotification({
			type: 'success',
			title: 'Petición enviada',
			text: `Si ${user.username} acepta, recibirás una invitación en «Chat y partidas».`,
		})
	} catch (cause) {
		addNotification({ type: 'error', title: 'No se pudo pedir unirse', text: typeof cause === 'string' ? cause : 'Inténtalo de nuevo.' })
	}
}
/** Abre «Chat y partidas» directamente con la conversación de este amigo. */
function openChat(id: string) {
	void router.push({ path: '/friends', query: { chat: id } })
}
const { avatarOf } = useFriendAvatars()
const { presence } = useFriendPresence(computed(() => social.value.friends.map((friend) => friend.id)))
// Quien tiene Strata abierto va primero; dentro de cada grupo, por orden alfabético.
const byName = (a: { username: string }, b: { username: string }) =>
	a.username.localeCompare(b.username, undefined, { sensitivity: 'base' })
const playingFriends = computed(() =>
	social.value.friends.filter((f) => presence.value[f.id]?.playing).sort(byName),
)
const isBusy = (id: string) => !!presence.value[id]?.busy
const onlineFriends = computed(() =>
	social.value.friends.filter((f) => presence.value[f.id]?.online && !presence.value[f.id]?.playing).sort(byName),
)
const offlineFriends = computed(() => social.value.friends.filter((f) => !presence.value[f.id]?.online).sort(byName))
const groups = computed(() => [
	{ key: 'playing' as const, label: 'Jugando', users: playingFriends.value },
	{ key: 'online' as const, label: 'En línea', users: onlineFriends.value },
	{ key: 'offline' as const, label: 'Desconectados', users: offlineFriends.value },
])
/** «Hace 12 min», «hace 2 h»: cuánto lleva jugando. */
function sinceText(seconds: number) {
	const minutes = Math.max(0, Math.round((Date.now() / 1000 - seconds) / 60))
	if (minutes < 1) return 'ahora mismo'
	if (minutes < 60) return `desde hace ${minutes} min`
	return `desde hace ${Math.floor(minutes / 60)} h`
}

const addModal = ref<InstanceType<typeof ModalWrapper> | null>(null)
const query = ref('')
const results = ref<SearchHit[]>([])
const searching = ref(false)
const searchError = ref('')
const searched = ref(false)
let timer: ReturnType<typeof setTimeout> | undefined
let searchId = 0

function openAdd() {
	query.value = ''
	results.value = []
	searchError.value = ''
	searched.value = false
	addModal.value?.show()
}
defineExpose({ openAdd })

watch(query, (value) => {
	clearTimeout(timer)
	searchError.value = ''
	const text = value.trim()
	if (text.length < 2) {
		results.value = []
		searched.value = false
		return
	}
	timer = setTimeout(async () => {
		const mine = ++searchId
		searching.value = true
		try {
			const found = (await account.search(text)).users
			if (mine !== searchId) return
			results.value = found
			searched.value = true
		} catch (cause) {
			if (mine === searchId)
				searchError.value = typeof cause === 'string' ? cause : 'No se pudo buscar.'
		} finally {
			if (mine === searchId) searching.value = false
		}
	}, 300)
})
onBeforeUnmount(() => clearTimeout(timer))

async function act(hit: SearchHit) {
	const ok = await account.friendAction(
		hit.relation === 'incoming' ? 'accept' : 'request',
		hit.username,
	)
	if (ok) {
		const relation: Relation = hit.relation === 'incoming' ? 'friend' : 'outgoing'
		results.value = results.value.map((item) => (item.id === hit.id ? { ...item, relation } : item))
	}
}
async function requestExact() {
	const name = query.value.trim()
	if (name && (await account.friendAction('request', name))) {
		query.value = ''
		addModal.value?.hide()
	}
}
const exactInResults = () =>
	results.value.some((hit) => hit.username.toLowerCase() === query.value.trim().toLowerCase())
</script>

<template>
	<ModalWrapper ref="addModal" header="Agrega a un amigo">
		<div class="min-w-[30rem] max-w-full">
			<h2 class="m-0 text-base font-medium text-primary">¿Cómo se llama tu amigo en Strata?</h2>
			<p class="m-0 mt-1 text-sm leading-tight text-secondary">
				Escribe al menos dos letras y elígelo de la lista.
			</p>
			<Input
				v-model="query"
				:icon="SearchIcon"
				type="text"
				placeholder="Busca por nombre de usuario…"
				autocomplete="off"
				:maxlength="24"
				wrapper-class="mt-4"
				clearable
			/>

			<p v-if="searchError" class="m-0 mt-3 text-sm text-red">{{ searchError }}</p>
			<p v-else-if="error" class="m-0 mt-3 text-sm text-red">{{ error }}</p>

			<div class="mt-3 flex max-h-72 flex-col gap-1 overflow-y-auto">
				<div
					v-for="hit in results"
					:key="hit.id"
					class="flex items-center gap-3 rounded-xl bg-surface-4 p-2 pl-3"
				>
					<UserAvatar :name="hit.username" :size="36" />
					<span class="min-w-0 flex-1 truncate font-semibold text-contrast">{{
						hit.username
					}}</span>
					<span v-if="hit.relation === 'friend'" class="text-sm text-secondary"
						>Ya sois amigos</span
					>
					<span v-else-if="hit.relation === 'outgoing'" class="text-sm text-secondary"
						>Solicitud enviada</span
					>
					<Button v-else type="colored" color="brand" size="sm" :disabled="busy" @click="act(hit)">
						<component :is="hit.relation === 'incoming' ? CheckIcon : UserPlusIcon" />
						{{ hit.relation === 'incoming' ? 'Aceptar' : 'Enviar solicitud' }}
					</Button>
				</div>
				<p v-if="searching && !results.length" class="m-0 py-3 text-center text-sm text-secondary">
					Buscando…
				</p>
				<p
					v-else-if="searched && !results.length && !searching"
					class="m-0 py-3 text-center text-sm text-secondary"
				>
					No hay nadie que empiece por «{{ query.trim() }}».
				</p>
			</div>

			<div
				v-if="query.trim().length >= 3 && !exactInResults() && !searching"
				class="mt-3 text-sm text-secondary"
			>
				¿No aparece? Quien se oculta del buscador solo recibe solicitudes si escribes su nombre
				exacto.
				<Button type="outlined" size="sm" class="mt-2" :disabled="busy" @click="requestExact">
					<UserPlusIcon /> Enviar solicitud a «{{ query.trim() }}»
				</Button>
			</div>
		</div>
	</ModalWrapper>

	<ModalWrapper ref="reportModal" :header="`Denunciar a ${reportTarget}`">
		<div class="flex min-w-[26rem] max-w-full flex-col gap-3">
			<p class="m-0 text-sm text-secondary">
				Cuéntanos qué ha pasado (hasta 300 caracteres). Solo se pueden denunciar amigos y solicitudes recibidas.
			</p>
			<Textarea v-model="reportReason" :maxlength="300" :rows="4" aria-label="Motivo de la denuncia" />
			<p v-if="error" class="m-0 text-sm text-red" role="alert">{{ error }}</p>
			<div class="flex justify-end gap-2">
				<Button type="outlined" @click="reportModal?.hide()">Cancelar</Button>
				<Button type="colored" color="red" :disabled="busy || !reportReason.trim()" @click="sendReport">Enviar denuncia</Button>
			</div>
		</div>
	</ModalWrapper>

	<div v-if="!loggedIn" class="text-sm">
		<span v-if="status?.offline" class="text-secondary"
			>No se pudo conectar con el servidor de cuentas.</span
		>
		<template v-else>
			<span class="font-semibold text-brand cursor-pointer" @click="emit('signIn')"
				>Crea tu cuenta de Strata</span
			>
			para añadir amigos y jugar con ellos.
		</template>
	</div>

	<div v-else class="flex flex-col gap-3">
		<button
			v-if="profile"
			type="button"
			class="flex w-full cursor-pointer items-center gap-3 rounded-xl border border-solid border-surface-5 bg-surface-3 p-2.5 text-left transition-colors hover:bg-surface-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
			aria-label="Editar tu perfil"
			@click="emit('editProfile')"
		>
			<UserAvatar :name="profile.username" :src="avatar" :size="44" :status="status?.offline ? 'offline' : 'online'" />
			<span class="min-w-0 flex-1">
				<span class="block truncate font-bold leading-tight text-contrast">{{ profile.username }}</span>
				<span class="block truncate text-xs leading-tight text-secondary">
					{{ status?.offline ? 'Sin conexión con el servidor' : 'En línea · Editar perfil' }}
				</span>
			</span>
		</button>
		<label class="flex cursor-pointer items-center justify-between gap-2 px-1 text-sm text-secondary">
			<span>No molestar <span class="text-xs">(sin peticiones ni «jugando a»)</span></span>
			<Toggle :model-value="doNotDisturb" @update:model-value="setDoNotDisturb" />
		</label>
		<div class="flex items-center gap-1 -ml-1">
			<h3 class="m-0 flex-1 pl-1 text-base font-medium text-primary">
				Amigos · {{ social.friends.length }}
			</h3>
			<IconButton
				v-tooltip="'Añadir un amigo'"
				type="quiet"
				label="Añadir un amigo"
				@click="openAdd"
			>
				<UserPlusIcon />
			</IconButton>
		</div>

		<div
			v-for="request in joinRequests"
			:key="request.id"
			class="flex flex-col gap-2 rounded-xl border border-solid border-brand bg-brand-highlight p-2.5"
			role="group"
			:aria-label="`${nameOf(request.friend)} quiere unirse a tu partida`"
		>
			<div class="flex items-center gap-2">
				<UserAvatar :name="nameOf(request.friend)" :size="32" />
				<span class="min-w-0 flex-1 text-sm leading-tight text-contrast">
					<strong>{{ nameOf(request.friend) }}</strong> quiere unirse a tu partida
				</span>
			</div>
			<Chips
				v-if="openShares.length > 1"
				:model-value="chosenShare[request.friend] ?? openShares[0].id"
				:items="openShares.map((share) => share.id)"
				:format-label="(id: string) => openShares.find((share) => share.id === id)?.title ?? id"
				:capitalize="false"
				aria-label="Partida a la que invitar"
				@update:model-value="(id: string | null | undefined) => id && (chosenShare[request.friend] = id)"
			/>
			<div class="flex gap-2">
				<Button type="colored" color="brand" size="sm" class="flex-1" @click="answerJoin(request.friend, true)">
					<CheckIcon /> Invitar
				</Button>
				<Button type="outlined" size="sm" class="flex-1" @click="answerJoin(request.friend, false)">
					<XIcon /> Rechazar
				</Button>
			</div>
		</div>
		<p v-if="joinError" class="m-0 text-sm text-red" role="alert">{{ joinError }}</p>

		<template v-if="social.incoming.length">
			<div class="text-sm font-semibold text-contrast">
				Solicitudes recibidas · {{ social.incoming.length }}
			</div>
			<div
				v-for="user in social.incoming"
				:key="user.id"
				class="flex items-center gap-2 rounded-xl bg-surface-3 p-2"
			>
				<UserAvatar :name="user.username" :src="avatarOf(user)" :size="32" />
				<span class="min-w-0 flex-1 truncate font-semibold text-contrast">{{ user.username }}</span>
				<IconButton
					v-tooltip="'Aceptar'"
					type="colored"
					color="brand"
					label="Aceptar"
					:disabled="busy"
					@click="account.friendAction('accept', user.username)"
				>
					<CheckIcon />
				</IconButton>
				<IconButton
					v-tooltip="'Rechazar'"
					type="quiet"
					label="Rechazar"
					:disabled="busy"
					@click="account.friendAction('remove', user.username)"
				>
					<XIcon />
				</IconButton>
				<IconButton
					v-tooltip="'Bloquear'"
					type="quiet"
					label="Bloquear"
					:disabled="busy"
					@click="account.friendAction('block', user.username)"
				>
					<BanIcon />
				</IconButton>
			</div>
		</template>

		<p v-if="!social.friends.length && !social.incoming.length" class="m-0 text-sm text-secondary">
			Aún no tienes amigos.
			<span class="cursor-pointer font-semibold text-brand" @click="openAdd"
				>Búscalos por su nombre</span
			>
			para jugar juntos.
		</p>
		<template v-for="group in groups" :key="group.key">
			<div v-if="group.users.length" class="flex flex-col gap-0.5">
				<div class="px-1 pb-1 text-xs font-semibold uppercase tracking-wide text-secondary">
					{{ group.label }} · {{ group.users.length }}
				</div>
				<div
					v-for="user in group.users"
					:key="user.id"
					class="group flex items-center gap-2.5 rounded-xl p-1.5 hover:bg-surface-3 focus-within:bg-surface-3"
				>
					<button
						type="button"
						class="flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 rounded-lg border-0 bg-transparent p-0 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
						:aria-label="`Abrir chat con ${user.username}`"
						@click="openChat(user.id)"
					>
					<UserAvatar
						:name="user.username"
						:src="avatarOf(user)"
						:size="36"
						:status="
							group.key === 'offline'
								? 'offline'
								: isBusy(user.id)
									? 'busy'
									: group.key === 'playing'
										? 'playing'
										: 'online'
						"
					/>
					<div class="min-w-0 flex-1" :title="user.bio || undefined">
						<div
							class="truncate font-semibold leading-tight"
							:class="group.key === 'offline' ? 'text-secondary' : 'text-contrast'"
						>
							{{ user.username }}
						</div>
						<div
							class="truncate text-xs leading-tight"
							:class="group.key === 'playing' ? 'text-brand' : 'text-secondary'"
							:title="group.key === 'playing' ? `${presence[user.id]?.playing?.title} · ${sinceText(presence[user.id]?.playing?.since ?? 0)}` : undefined"
						>
							<template v-if="group.key === 'playing' && presence[user.id]?.playing">
								Jugando a {{ presence[user.id]?.playing?.title }}
							</template>
							<template v-else-if="group.key === 'online' && isBusy(user.id)">No molestar</template>
							<template v-else>{{ group.key === 'online' ? 'En línea' : 'Desconectado' }}</template>
						</div>
					</div>
					</button>
					<TeleportOverflowMenu
						type="quiet"
						:label="`Más acciones para ${user.username}`"
						class="shrink-0 opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
						:options="[
							{ id: 'chat', label: 'Enviar mensaje', action: () => openChat(user.id) },
							...(group.key === 'offline' || isBusy(user.id)
								? []
								: [{ id: 'join', label: 'Pedir unirse a su partida', action: () => askToJoin(user) }]),
							{ id: 'report', label: 'Denunciar', action: () => openReport(user.username) },
							{ id: 'remove', label: 'Eliminar amigo', action: () => account.friendAction('remove', user.username) },
							{ id: 'block', label: 'Bloquear', action: () => account.friendAction('block', user.username), tone: 'red' },
						]"
					>
						<MoreVerticalIcon />
						<template #chat><SendIcon /> Enviar mensaje</template>
						<template #join><PlayIcon /> Pedir unirse a su partida</template>
						<template #report><ReportIcon /> Denunciar</template>
						<template #remove><XIcon /> Eliminar amigo</template>
						<template #block><BanIcon /> Bloquear</template>
					</TeleportOverflowMenu>
				</div>
			</div>
		</template>

		<template v-if="social.outgoing.length">
			<div class="text-sm font-semibold text-contrast">
				Pendientes · {{ social.outgoing.length }}
			</div>
			<div v-for="user in social.outgoing" :key="user.id" class="flex items-center gap-2 p-1">
				<span class="min-w-0 flex-1 truncate text-primary">{{ user.username }}</span>
				<Button
					type="outlined"
					size="sm"
					:disabled="busy"
					@click="account.friendAction('remove', user.username)"
					>Cancelar</Button
				>
			</div>
		</template>

		<Button type="outlined" size="sm" @click="router.push('/friends')"
			><UsersIcon /> Chat y partidas</Button
		>
	</div>
</template>
