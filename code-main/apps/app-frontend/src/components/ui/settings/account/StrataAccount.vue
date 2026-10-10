<script setup lang="ts">
import { CheckIcon, CopyIcon, LogInIcon, LogOutIcon, TrashIcon, UserIcon } from '@modrinth/assets'
import { Admonition, Button, Input, Textarea } from '@modrinth/ui'
import { ask } from '@tauri-apps/plugin-dialog'
import { computed, ref, watch } from 'vue'

import { useStrataAccount } from '@/composables/use-strata-account'

const account = useStrataAccount()
const { status, profile, loggedIn, busy, error } = account

const mode = ref<'login' | 'register'>('login')
const username = ref('')
const password = ref('')
const bio = ref(profile.value?.bio ?? '')
const copied = ref(false)
const showSecurity = ref(false)
const showServer = ref(false)
const currentPassword = ref('')
const newPassword = ref('')
const deletePassword = ref('')
const serverUrl = ref('')
const notice = ref('')

watch(profile, (value) => {
	bio.value = value?.bio ?? ''
})
watch(
	() => status.value?.server_url,
	(url) => {
		if (url && !serverUrl.value) serverUrl.value = url
	},
	{ immediate: true },
)

const changed = computed(() => !!profile.value && bio.value !== profile.value.bio)
defineExpose({
	hasChanges: changed,
	originalState: computed(() => ({ bio: profile.value?.bio ?? '' })),
	modifiedState: computed(() => ({ bio: bio.value })),
	saving: busy,
	reset: () => {
		bio.value = profile.value?.bio ?? ''
	},
	save: () => account.update({ bio: bio.value }),
})

async function submit() {
	if (!username.value.trim() || !password.value) return
	const action = mode.value === 'login' ? account.login : account.register
	if (await action(username.value.trim(), password.value)) password.value = ''
}
async function copyCode() {
	if (!profile.value?.p2p_code) return
	try {
		await navigator.clipboard.writeText(profile.value.p2p_code)
		copied.value = true
		setTimeout(() => (copied.value = false), 2000)
	} catch {
		copied.value = false
	}
}
async function changePassword() {
	if (await account.changePassword(currentPassword.value, newPassword.value)) {
		currentPassword.value = ''
		newPassword.value = ''
		notice.value = 'Contraseña cambiada. Se han cerrado tus otras sesiones.'
	}
}
async function deleteAccount() {
	if (!deletePassword.value) return
	const confirmed = await ask(
		'¿Eliminar tu cuenta de Strata? Se borrarán tu nombre, tus amigos y tus bloqueos. No se puede deshacer.',
		{ title: 'Eliminar cuenta', kind: 'warning', okLabel: 'Eliminar', cancelLabel: 'Cancelar' },
	)
	if (confirmed && (await account.deleteAccount(deletePassword.value))) deletePassword.value = ''
}
async function saveServer() {
	if (await account.setServer(serverUrl.value)) {
		serverUrl.value = status.value?.server_url ?? ''
		notice.value = 'Servidor cambiado. Inicia sesión de nuevo.'
	}
}
</script>

<template>
	<section class="flex flex-col gap-4">
		<h2 class="m-0 flex items-center gap-2 text-xl font-bold text-contrast">
			<UserIcon class="size-6 text-brand" /> Cuenta de Strata
		</h2>

		<Admonition
			v-if="error"
			type="critical"
			:body="error"
			dismissible
			@dismiss="account.clearError()"
		/>
		<Admonition
			v-else-if="notice"
			type="success"
			:body="notice"
			dismissible
			@dismiss="notice = ''"
		/>
		<Admonition
			v-if="status?.offline"
			type="warning"
			body="No se pudo conectar con el servidor de cuentas. Tu sesión sigue guardada; se reintentará solo."
		/>

		<!-- Sin sesión -->
		<div
			v-if="!loggedIn"
			class="flex flex-col gap-4 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
		>
			<p class="m-0 text-secondary">
				Con tu cuenta reservas un nombre de usuario único, tus amigos pueden encontrarte en el
				buscador y podéis chatear y jugar juntos.
			</p>
			<div class="flex gap-2">
				<Button
					:type="mode === 'login' ? 'colored' : 'outlined'"
					:color="mode === 'login' ? 'brand' : undefined"
					size="sm"
					@click="mode = 'login'"
				>
					Iniciar sesión
				</Button>
				<Button
					:type="mode === 'register' ? 'colored' : 'outlined'"
					:color="mode === 'register' ? 'brand' : undefined"
					size="sm"
					@click="mode = 'register'"
				>
					Crear cuenta
				</Button>
			</div>
			<form class="flex flex-col gap-3" @submit.prevent="submit">
				<div class="flex flex-col gap-1.5">
					<label for="acc-user" class="font-semibold text-contrast">Nombre de usuario</label>
					<Input id="acc-user" v-model="username" :maxlength="24" autocomplete="username" />
					<span v-if="mode === 'register'" class="text-sm text-secondary">
						De 3 a 24 letras, números o guiones bajos. Nadie más podrá usar el mismo nombre (sin
						distinguir mayúsculas).
					</span>
				</div>
				<div class="flex flex-col gap-1.5">
					<label for="acc-pass" class="font-semibold text-contrast">Contraseña</label>
					<Input
						id="acc-pass"
						v-model="password"
						type="password"
						:maxlength="128"
						:autocomplete="mode === 'login' ? 'current-password' : 'new-password'"
					/>
					<span v-if="mode === 'register'" class="text-sm text-secondary">
						Mínimo 12 caracteres. No hay recuperación de contraseña: guárdala en un lugar seguro.
					</span>
				</div>
				<div>
					<Button
						type="colored"
						color="brand"
						native-type="submit"
						:disabled="busy || !username.trim() || !password"
					>
						<LogInIcon /> {{ mode === 'login' ? 'Iniciar sesión' : 'Crear cuenta' }}
					</Button>
				</div>
			</form>
		</div>

		<!-- Con sesión -->
		<template v-else-if="profile">
			<div
				class="flex flex-wrap items-center gap-3 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<div
					class="flex size-12 shrink-0 items-center justify-center rounded-full bg-surface-5 text-xl font-bold text-contrast"
				>
					{{ profile.username[0]?.toUpperCase() }}
				</div>
				<div class="min-w-0 flex-1">
					<div class="truncate text-lg font-bold text-contrast">{{ profile.username }}</div>
					<div class="text-sm text-secondary">
						Tu nombre de usuario es único y no se puede repetir.
					</div>
				</div>
				<Button type="outlined" :disabled="busy" @click="account.logout()"
					><LogOutIcon /> Cerrar sesión</Button
				>
			</div>

			<div
				class="flex flex-col gap-4 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<div class="flex flex-col gap-1.5">
					<label for="acc-bio" class="font-semibold text-contrast">Biografía</label>
					<Textarea id="acc-bio" v-model="bio" :maxlength="500" :rows="3" />
					<span class="text-sm text-secondary">La ven tus amigos.</span>
				</div>
				<div>
					<Button
						type="colored"
						color="brand"
						:disabled="busy || !changed"
						@click="account.update({ bio })"
					>
						<CheckIcon /> Guardar cambios
					</Button>
				</div>
			</div>

			<div
				class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<h3 class="m-0 text-lg font-bold text-contrast">Chat y partidas</h3>
				<p class="m-0 text-secondary">
					Tu launcher se enlaza con tu cuenta con una clave pública. Tus amigos la reciben solos del
					servidor: no tienes que pasar códigos a mano.
				</p>
				<div class="flex gap-2">
					<Input
						:model-value="profile.p2p_code ?? 'Enlazando con tu cuenta…'"
						readonly
						wrapper-class="flex-grow min-w-0"
					/>
					<Button type="outlined" :disabled="!profile.p2p_code" @click="copyCode">
						<CheckIcon v-if="copied" /><CopyIcon v-else />
						{{ copied ? 'Copiado' : 'Copiar' }}
					</Button>
				</div>
				<span class="text-sm text-secondary">
					Solo te sirve si quieres añadir a alguien sin cuenta. Esta clave no da acceso a tu cuenta
					de Minecraft ni a tus archivos.
				</span>
			</div>

			<div
				class="flex flex-col gap-3 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<button
					class="flex cursor-pointer items-center justify-between border-0 bg-transparent p-0 text-left"
					@click="showSecurity = !showSecurity"
				>
					<h3 class="m-0 text-lg font-bold text-contrast">Seguridad</h3>
					<span class="text-sm text-secondary">{{ showSecurity ? 'Ocultar' : 'Mostrar' }}</span>
				</button>
				<template v-if="showSecurity">
					<div class="flex flex-col gap-2">
						<label class="font-semibold text-contrast" for="acc-cur">Cambiar contraseña</label>
						<Input
							id="acc-cur"
							v-model="currentPassword"
							type="password"
							placeholder="Contraseña actual"
							autocomplete="current-password"
						/>
						<Input
							v-model="newPassword"
							type="password"
							placeholder="Contraseña nueva (mínimo 12 caracteres)"
							autocomplete="new-password"
						/>
						<div>
							<Button
								type="outlined"
								:disabled="busy || !currentPassword || newPassword.length < 12"
								@click="changePassword"
							>
								Cambiar contraseña
							</Button>
						</div>
					</div>
					<div class="flex flex-col gap-2">
						<label class="font-semibold text-contrast" for="acc-del">Eliminar cuenta</label>
						<span class="text-sm text-secondary">
							Borra tu nombre, tus amigos y tus bloqueos. Tu nombre quedará libre para otras
							personas.
						</span>
						<Input
							id="acc-del"
							v-model="deletePassword"
							type="password"
							placeholder="Tu contraseña"
							autocomplete="current-password"
						/>
						<div>
							<Button
								type="outlined"
								color="red"
								:disabled="busy || !deletePassword"
								@click="deleteAccount"
							>
								<TrashIcon /> Eliminar cuenta
							</Button>
						</div>
					</div>
				</template>
			</div>
		</template>

		<div class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5">
			<button
				class="flex cursor-pointer items-center justify-between border-0 bg-transparent p-0 text-left"
				@click="showServer = !showServer"
			>
				<h3 class="m-0 text-lg font-bold text-contrast">Servidor de cuentas</h3>
				<span class="text-sm text-secondary">{{ showServer ? 'Ocultar' : 'Mostrar' }}</span>
			</button>
			<template v-if="showServer">
				<span class="text-sm text-secondary">
					Dirección del servicio que guarda los nombres de usuario y las amistades. Cambiarla cierra
					tu sesión. Deja el campo vacío para volver al servidor por defecto.
				</span>
				<div class="flex gap-2">
					<Input v-model="serverUrl" placeholder="https://…" wrapper-class="flex-grow min-w-0" />
					<Button type="outlined" :disabled="busy" @click="saveServer">Guardar</Button>
				</div>
				<span v-if="status && !status.default_server" class="text-sm text-secondary">
					Estás usando un servidor distinto del predeterminado.
				</span>
			</template>
		</div>
	</section>
</template>
