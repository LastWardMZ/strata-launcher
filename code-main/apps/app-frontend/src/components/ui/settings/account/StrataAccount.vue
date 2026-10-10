<script setup lang="ts">
import { CheckIcon, CopyIcon, UserIcon } from '@modrinth/assets'
import { Admonition, Button, Input, Textarea } from '@modrinth/ui'
import { computed, ref, watch } from 'vue'

import ModrinthConnection from '@/components/ui/settings/account/ModrinthConnection.vue'
import { useStrataAccount } from '@/composables/use-strata-account'

const { user, busy, error, saveProfile } = useStrataAccount()

const name = ref(user.value?.username || '')
const bio = ref(user.value?.bio || '')
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
</script>

<template>
	<section class="flex flex-col gap-4">
		<h2 class="m-0 flex items-center gap-2 text-xl font-bold text-contrast">
			<UserIcon class="size-6 text-brand" /> Perfil de Strata
		</h2>

		<ModrinthConnection />

		<Admonition v-if="error" type="critical" :body="error" />

		<div class="flex flex-col gap-4 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5">
			<div>
				<h3 class="m-0 text-lg font-bold text-contrast">Identidad para partidas</h3>
				<p class="m-0 mt-1 text-secondary">
					Con esta identidad se conectan los launchers entre sí para el chat y las partidas P2P. No
					necesitas registrarte ni crear contraseñas.
				</p>
			</div>

			<div class="flex flex-col gap-1.5">
				<label for="strata-name" class="font-semibold text-contrast">Nombre</label>
				<Input id="strata-name" v-model="name" :maxlength="32" autocomplete="off" />
			</div>

			<div class="flex flex-col gap-1.5">
				<label for="strata-code" class="font-semibold text-contrast">Tu código de partidas</label>
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
					Es tu clave pública: puedes dársela a quien quieras que juegue contigo. No da acceso a tu
					cuenta de Minecraft ni a tus archivos.
				</span>
			</div>

			<div class="flex flex-col gap-1.5">
				<label for="strata-bio" class="font-semibold text-contrast">Biografía</label>
				<Textarea id="strata-bio" v-model="bio" :maxlength="500" :rows="4" />
				<span class="text-sm text-secondary">Solo se guarda en este equipo.</span>
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
		</div>

		<p class="m-0 text-sm text-secondary">
			La cuenta de Microsoft para jugar a Minecraft se gestiona por separado.
		</p>
	</section>
</template>
