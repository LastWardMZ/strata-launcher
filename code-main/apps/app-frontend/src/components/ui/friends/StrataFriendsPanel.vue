<script setup lang="ts">
import { BanIcon, CheckIcon, SearchIcon, UserPlusIcon, UsersIcon, XIcon } from '@modrinth/assets'
import { Button, IconButton, Input } from '@modrinth/ui'
import { onBeforeUnmount, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import ModalWrapper from '@/components/ui/modal/ModalWrapper.vue'
import { type Relation, type SearchHit, useStrataAccount } from '@/composables/use-strata-account'

const emit = defineEmits<{ signIn: [] }>()
const router = useRouter()
const account = useStrataAccount()
const { loggedIn, social, busy, error, status } = account

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
const initial = (name: string) => (name.trim()[0] ?? '?').toUpperCase()
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
					<div
						class="flex size-9 shrink-0 items-center justify-center rounded-full bg-surface-5 font-bold text-contrast"
					>
						{{ initial(hit.username) }}
					</div>
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

		<template v-if="social.incoming.length">
			<div class="text-sm font-semibold text-contrast">
				Solicitudes recibidas · {{ social.incoming.length }}
			</div>
			<div
				v-for="user in social.incoming"
				:key="user.id"
				class="flex items-center gap-2 rounded-xl bg-surface-3 p-2"
			>
				<div
					class="flex size-8 shrink-0 items-center justify-center rounded-full bg-surface-5 text-sm font-bold text-contrast"
				>
					{{ initial(user.username) }}
				</div>
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
		<div
			v-for="user in social.friends"
			:key="user.id"
			class="group flex items-center gap-2 rounded-xl p-1 hover:bg-surface-3"
		>
			<div
				class="flex size-8 shrink-0 items-center justify-center rounded-full bg-surface-5 text-sm font-bold text-contrast"
			>
				{{ initial(user.username) }}
			</div>
			<span
				class="min-w-0 flex-1 truncate font-semibold text-contrast"
				:title="user.bio || undefined"
				>{{ user.username }}</span
			>
			<span class="hidden gap-1 group-hover:flex">
				<IconButton
					v-tooltip="'Eliminar amigo'"
					type="quiet"
					label="Eliminar amigo"
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
			</span>
		</div>

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
