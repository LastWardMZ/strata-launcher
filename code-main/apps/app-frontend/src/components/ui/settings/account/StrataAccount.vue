<script setup lang="ts">
import {
	CheckIcon,
	CopyIcon,
	DownloadIcon,
	TrashIcon,
	UploadIcon,
	UserIcon,
} from '@modrinth/assets'
import { Admonition, Button, Input, Textarea } from '@modrinth/ui'
import { confirm, open, save } from '@tauri-apps/plugin-dialog'
import { computed, ref, watch } from 'vue'

import UserAvatar from '@/components/ui/friends/UserAvatar.vue'
import { useProfileAvatar } from '@/composables/use-profile-avatar'
import { useStrataAccount } from '@/composables/use-strata-account'
import { restartApp } from '@/helpers/utils'

const account = useStrataAccount()
const { status, profile, registered, busy, error } = account

const username = ref('')
const newName = ref('')
const bio = ref(profile.value?.bio ?? '')
const copied = ref(false)
const showSecurity = ref(false)
const showServer = ref(false)
const showRename = ref(false)
const serverUrl = ref('')
const notice = ref('')
const { avatar, setFromFile, clear: clearAvatar } = useProfileAvatar()
const photoInput = ref<HTMLInputElement | null>(null)
const photoError = ref('')

async function pickPhoto(event: Event) {
	const input = event.target as HTMLInputElement
	const file = input.files?.[0]
	input.value = ''
	if (!file) return
	photoError.value = ''
	try {
		await setFromFile(file)
		notice.value = 'Foto actualizada.'
	} catch (cause) {
		photoError.value = cause instanceof Error ? cause.message : 'No se pudo usar esa imagen.'
	}
}

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

async function createAccount() {
	if (!username.value.trim()) return
	if (await account.register(username.value.trim())) username.value = ''
}
async function rename() {
	if (newName.value.trim() && (await account.rename(newName.value.trim()))) {
		newName.value = ''
		showRename.value = false
		notice.value = 'Nombre cambiado.'
	}
}
async function copyCode() {
	if (!profile.value) return
	try {
		await navigator.clipboard.writeText(profile.value.id)
		copied.value = true
		setTimeout(() => (copied.value = false), 2000)
	} catch {
		copied.value = false
	}
}
async function deleteAccount() {
	const confirmed = await confirm(
		'¿Eliminar tu cuenta de Strata? Se borrarán tu nombre, tus amigos y tus bloqueos, y tu nombre quedará libre para otras personas. No se puede deshacer.',
		{ title: 'Eliminar cuenta', kind: 'warning', okLabel: 'Eliminar', cancelLabel: 'Cancelar' },
	)
	if (confirmed) await account.deleteAccount()
}
async function exportIdentity() {
	const path = await save({
		defaultPath: 'strata-copia-de-seguridad.key',
		title: 'Guardar copia de seguridad',
	})
	if (path && (await account.exportIdentity(path))) {
		notice.value =
			'Copia guardada. Guárdala en un sitio seguro y no la compartas: quien la tenga es tú.'
	}
}
async function importIdentity() {
	const selected = await open({ multiple: false, title: 'Elegir copia de seguridad' })
	const path = Array.isArray(selected) ? selected[0] : selected
	if (!path) return
	const confirmed = await confirm(
		'Este launcher pasará a usar la identidad de la copia (tu cuenta y tu código cambiarán) y se reiniciará. La identidad actual se guarda por si acaso. ¿Continuar?',
		{
			title: 'Restaurar copia de seguridad',
			kind: 'warning',
			okLabel: 'Restaurar y reiniciar',
			cancelLabel: 'Cancelar',
		},
	)
	if (confirmed && (await account.importIdentity(path))) await restartApp()
}
async function saveServer() {
	if (await account.setServer(serverUrl.value)) {
		serverUrl.value = status.value?.server_url ?? ''
		notice.value = 'Servidor cambiado.'
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
			body="No se pudo conectar con el servidor de cuentas. Se reintentará solo."
		/>
		<Admonition v-else-if="status?.problem" type="warning" :body="status.problem" />

		<!-- Sin cuenta -->
		<div
			v-if="!registered && !status?.offline"
			class="flex flex-col gap-4 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
		>
			<p class="m-0 text-secondary">
				Crea tu cuenta para reservar un nombre de usuario único, que tus amigos te encuentren en el
				buscador y podáis chatear y jugar juntos. No hay contraseña: tu cuenta es la clave de este
				launcher, así que conviene que hagas una copia de seguridad (más abajo).
			</p>
			<form class="flex flex-col gap-3" @submit.prevent="createAccount">
				<div class="flex flex-col gap-1.5">
					<label for="acc-user" class="font-semibold text-contrast">Nombre de usuario</label>
					<Input id="acc-user" v-model="username" :maxlength="24" autocomplete="off" />
					<span class="text-sm text-secondary">
						De 3 a 24 letras, números o guiones bajos. Nadie más podrá usar el mismo nombre (sin
						distinguir mayúsculas).
					</span>
				</div>
				<div>
					<Button
						type="colored"
						color="brand"
						native-type="submit"
						:disabled="busy || !username.trim()"
					>
						<CheckIcon /> Crear cuenta
					</Button>
				</div>
			</form>
		</div>

		<!-- Con cuenta -->
		<template v-else-if="profile">
			<div
				class="flex flex-wrap items-center gap-3 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<UserAvatar :name="profile.username" :src="avatar" :size="72" />
				<div class="min-w-0 flex-1">
					<div class="truncate text-lg font-bold text-contrast">{{ profile.username }}</div>
					<div class="text-sm text-secondary">
						Tu nombre de usuario es único y no se puede repetir.
					</div>
					<div class="mt-2 flex flex-wrap gap-2">
						<input
							ref="photoInput"
							type="file"
							accept="image/png,image/jpeg,image/webp,image/gif"
							class="hidden"
							@change="pickPhoto"
						/>
						<Button size="sm" @click="photoInput?.click()"><UploadIcon /> {{ avatar ? 'Cambiar foto' : 'Subir foto' }}</Button>
						<Button v-if="avatar" size="sm" type="outlined" @click="clearAvatar"><TrashIcon /> Quitar foto</Button>
					</div>
					<div v-if="photoError" class="mt-1 text-sm text-red" role="alert">{{ photoError }}</div>
					<div v-else class="mt-1 text-sm text-secondary">
						La foto se recorta a un cuadrado y se guarda en este equipo; todavía no se comparte con tus amigos.
					</div>
				</div>
				<Button type="outlined" @click="showRename = !showRename">Cambiar nombre</Button>
			</div>
			<div
				v-if="showRename"
				class="flex gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
			>
				<Input
					v-model="newName"
					:maxlength="24"
					placeholder="Nuevo nombre"
					autocomplete="off"
					wrapper-class="flex-grow min-w-0"
					@keyup.enter="rename"
				/>
				<Button type="colored" color="brand" :disabled="busy || !newName.trim()" @click="rename"
					>Cambiar</Button
				>
				<span class="self-center text-sm text-secondary">Una vez al día.</span>
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
					Tu cuenta es la clave pública de este launcher. Tus amigos la reciben solos del servidor:
					no tienes que pasar códigos a mano.
				</p>
				<div class="flex gap-2">
					<Input :model-value="profile.id" readonly wrapper-class="flex-grow min-w-0" />
					<Button type="outlined" @click="copyCode">
						<CheckIcon v-if="copied" /><CopyIcon v-else />
						{{ copied ? 'Copiado' : 'Copiar' }}
					</Button>
				</div>
				<span class="text-sm text-secondary">
					Solo te hace falta si quieres añadir a alguien que no tiene cuenta. No da acceso a tu
					cuenta de Minecraft ni a tus archivos.
				</span>
			</div>
		</template>

		<div
			v-if="!status?.offline"
			class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
		>
			<h3 class="m-0 text-lg font-bold text-contrast">Copia de seguridad de tu cuenta</h3>
			<p class="m-0 text-secondary">
				Tu cuenta vive en un archivo de este equipo y
				<strong class="text-contrast">no se puede recuperar si se pierde</strong> (no hay contraseña
				ni correo). Guarda una copia, y úsala para llevar tu cuenta a otro ordenador.
			</p>
			<div class="flex flex-wrap gap-2">
				<Button type="outlined" :disabled="busy" @click="exportIdentity"
					><DownloadIcon /> Guardar copia</Button
				>
				<Button type="outlined" :disabled="busy" @click="importIdentity"
					><UploadIcon /> Restaurar copia</Button
				>
			</div>
			<span class="text-sm text-secondary">
				La copia da acceso total a tu cuenta: no la compartas ni la subas a ningún sitio público.
			</span>
		</div>

		<div
			v-if="registered"
			class="flex flex-col gap-3 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
		>
			<button
				class="flex cursor-pointer items-center justify-between border-0 bg-transparent p-0 text-left"
				@click="showSecurity = !showSecurity"
			>
				<h3 class="m-0 text-lg font-bold text-contrast">Eliminar cuenta</h3>
				<span class="text-sm text-secondary">{{ showSecurity ? 'Ocultar' : 'Mostrar' }}</span>
			</button>
			<template v-if="showSecurity">
				<span class="text-sm text-secondary">
					Borra tu nombre, tus amigos y tus bloqueos del servidor, y libera tu nombre. Tu clave del
					launcher no cambia.
				</span>
				<div>
					<Button type="outlined" color="red" :disabled="busy" @click="deleteAccount"
						><TrashIcon /> Eliminar cuenta</Button
					>
				</div>
			</template>
		</div>

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
					Dirección del servicio que guarda los nombres de usuario y las amistades. Cambiarla retira
					de este launcher los amigos de la cuenta actual. Deja el campo vacío para volver al
					servidor por defecto.
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
