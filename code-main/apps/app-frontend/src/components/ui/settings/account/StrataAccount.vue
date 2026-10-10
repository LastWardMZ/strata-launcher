<script setup lang="ts">
import {
	BanIcon,
	CheckIcon,
	CopyIcon,
	HeartHandshakeIcon,
	UserIcon,
	UserPlusIcon,
	UsersIcon,
	XIcon,
} from '@modrinth/assets'
import { Admonition, Button, Input, Textarea } from '@modrinth/ui'
import { computed, ref, watch } from 'vue'

import { useStrataAccount } from '@/composables/use-strata-account'

defineProps<{ socialMode?: boolean; compact?: boolean }>()
const { user, social, busy, error, saveProfile, refresh, socialAction } = useStrataAccount()

const name = ref(user.value?.username || '')
const bio = ref(user.value?.bio || '')
const friendCode = ref('')
const copied = ref(false)

watch(user, (value) => {
	name.value = value?.username || ''
	bio.value = value?.bio || ''
})

const changed = computed(
	() => !!user.value && (name.value.trim() !== user.value.username || bio.value !== user.value.bio),
)
defineExpose({
	hasChanges: changed,
	originalState: computed(() => ({ name: user.value?.username || '', bio: user.value?.bio || '' })),
	modifiedState: computed(() => ({ name: name.value, bio: bio.value })),
	saving: busy,
	reset: () => {
		name.value = user.value?.username || ''
		bio.value = user.value?.bio || ''
	},
	save: () => saveProfile(name.value, bio.value),
})

async function copyCode() {
	if (!user.value) return
	try {
		await navigator.clipboard.writeText(user.value.id)
		copied.value = true
		setTimeout(() => (copied.value = false), 2000)
	} catch {
		copied.value = false
	}
}
async function addFriend() {
	const code = friendCode.value.trim()
	if (!code) return
	await socialAction('friends/request', code)
	if (!error.value) friendCode.value = ''
}

type Action = { id: string; label: string; red?: boolean }
const groups = computed(() => [
	{
		label: 'Amigos',
		users: social.value.friends,
		actions: [
			{ id: 'friends/remove', label: 'Eliminar' },
			{ id: 'block', label: 'Bloquear', red: true },
		] as Action[],
	},
	{
		label: 'Solicitudes recibidas',
		users: social.value.incoming,
		actions: [
			{ id: 'friends/accept', label: 'Aceptar' },
			{ id: 'friends/remove', label: 'Rechazar' },
		] as Action[],
	},
	{
		label: 'Solicitudes enviadas',
		users: social.value.outgoing,
		actions: [{ id: 'friends/remove', label: 'Cancelar' }] as Action[],
	},
	{
		label: 'Usuarios bloqueados',
		users: social.value.blocked,
		actions: [{ id: 'unblock', label: 'Desbloquear' }] as Action[],
	},
])
</script>

<template>
	<section class="flex flex-col gap-4" :class="compact ? 'p-4' : ''">
		<h2 class="m-0 flex items-center gap-2 text-xl font-bold text-contrast">
			<HeartHandshakeIcon v-if="socialMode" class="size-6 text-brand" />
			<UserIcon v-else class="size-6 text-brand" />
			{{ socialMode ? 'Amigos de Strata' : 'Cuenta de Strata' }}
		</h2>

		<Admonition v-if="error" type="critical" :body="error" />

		<template v-if="!socialMode">
			<p class="m-0 text-secondary">
				Tu cuenta de Strata es la identidad de este launcher: no necesitas registrarte ni crear
				contraseñas. Tus amigos te encuentran con tu código.
			</p>

			<div class="flex flex-col gap-1.5">
				<label for="strata-name" class="font-semibold text-contrast">Nombre</label>
				<Input id="strata-name" v-model="name" :maxlength="32" autocomplete="off" />
			</div>

			<div class="flex flex-col gap-1.5">
				<label for="strata-code" class="font-semibold text-contrast">Tu código de amigo</label>
				<div class="flex gap-2">
					<Input
						id="strata-code"
						:model-value="user?.id || 'Iniciando…'"
						readonly
						wrapper-class="flex-grow min-w-0"
					/>
					<Button type="outlined" :disabled="!user" @click="copyCode">
						<CheckIcon v-if="copied" /><CopyIcon v-else />
						{{ copied ? 'Copiado' : 'Copiar' }}
					</Button>
				</div>
				<span class="text-sm text-secondary">
					Es tu clave pública: compártela con quien quieras que te añada. No da acceso a tu cuenta
					de Minecraft ni a tus archivos.
				</span>
			</div>

			<div class="flex flex-col gap-1.5">
				<label for="strata-bio" class="font-semibold text-contrast">Biografía</label>
				<Textarea id="strata-bio" v-model="bio" :maxlength="500" :rows="4" />
				<span class="text-sm text-secondary">
					Solo se guarda en este equipo; de momento no se muestra a tus amigos.
				</span>
			</div>

			<div>
				<Button
					type="colored"
					color="brand"
					:disabled="busy || !changed"
					@click="saveProfile(name, bio)"
				>
					<CheckIcon /> Guardar cambios
				</Button>
			</div>
		</template>

		<template v-else>
			<div class="flex flex-col gap-1.5">
				<label for="strata-add" class="font-semibold text-contrast">Añadir un amigo</label>
				<div class="flex gap-2">
					<Input
						id="strata-add"
						v-model="friendCode"
						placeholder="Pega su código de amigo"
						autocomplete="off"
						wrapper-class="flex-grow min-w-0"
						@keyup.enter="addFriend"
					/>
					<Button
						type="colored"
						color="brand"
						:disabled="busy || !friendCode.trim()"
						@click="addFriend"
					>
						<UserPlusIcon /> Enviar
					</Button>
				</div>
			</div>

			<div class="flex gap-2">
				<Button type="outlined" size="sm" :disabled="busy" @click="refresh">Actualizar</Button>
				<Button v-if="compact" type="outlined" size="sm" @click="$router.push('/friends')">
					<UsersIcon /> Chat y partidas
				</Button>
			</div>

			<div
				v-for="group in groups"
				:key="group.label"
				class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-3 ring-1 ring-inset ring-surface-5"
			>
				<h3 class="m-0 flex items-center gap-2 text-base font-bold text-contrast">
					{{ group.label }}
					<span class="rounded-full bg-surface-5 px-2 text-xs font-semibold">{{
						group.users.length
					}}</span>
				</h3>
				<p v-if="!group.users.length" class="m-0 text-sm text-secondary">
					No hay usuarios en esta lista.
				</p>
				<div
					v-for="member in group.users"
					:key="member.id"
					class="flex gap-2"
					:class="compact ? 'flex-col' : 'flex-wrap items-center'"
				>
					<div class="flex min-w-0 flex-1 items-center gap-2">
						<div
							class="flex size-8 shrink-0 items-center justify-center rounded-full bg-surface-5 text-sm font-bold text-contrast"
						>
							{{ (member.username.trim()[0] ?? '?').toUpperCase() }}
						</div>
						<strong class="min-w-0 flex-1 truncate text-contrast">{{ member.username }}</strong>
					</div>
					<div class="flex flex-wrap gap-1.5">
						<Button
							v-for="action in group.actions"
							:key="action.id"
							type="outlined"
							size="sm"
							:color="action.red ? 'red' : undefined"
							:disabled="busy"
							@click="socialAction(action.id, member.id)"
						>
							<BanIcon v-if="action.id === 'block'" /><XIcon
								v-else-if="action.id === 'friends/remove'"
							/>
							{{ action.label }}
						</Button>
					</div>
				</div>
			</div>
		</template>

		<p class="m-0 text-sm text-secondary">
			La cuenta de Microsoft para jugar a Minecraft se gestiona por separado.
		</p>
	</section>
</template>
