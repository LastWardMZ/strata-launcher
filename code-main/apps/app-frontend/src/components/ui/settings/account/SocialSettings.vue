<script setup lang="ts">
import { Admonition, Button, Chips } from '@modrinth/ui'
import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, ref } from 'vue'

import {
	type FriendPrivacy,
	type InvitePrivacy,
	useStrataAccount,
} from '@/composables/use-strata-account'

const account = useStrataAccount()
const { profile, loggedIn, social, busy, error } = account

const friendOptions: FriendPrivacy[] = ['everyone', 'mutual', 'none']
const friendLabels: Record<FriendPrivacy, string> = {
	everyone: 'Todos',
	mutual: 'Amigos de amigos',
	none: 'Nadie',
}
const inviteOptions: InvitePrivacy[] = ['friends', 'none']
const inviteLabels: Record<InvitePrivacy, string> = { friends: 'Amigos', none: 'Nadie' }
const searchableOptions = ['yes', 'no'] as const
const searchableLabels = { yes: 'Sí', no: 'No' }

const searchable = computed(() => (profile.value?.settings.searchable ? 'yes' : 'no'))

// Compartir a qué juegas es un ajuste local de este launcher (se decide aquí, no en el servidor de cuentas).
const shareActivity = ref<'yes' | 'no'>('yes')
const activityBusy = ref(false)
onMounted(async () => {
	try {
		shareActivity.value = (await invoke<boolean>('p2p_activity_sharing')) ? 'yes' : 'no'
	} catch {
		// Se queda en el valor por defecto.
	}
})
async function setShareActivity(value: 'yes' | 'no' | null) {
	if (!value || activityBusy.value) return
	activityBusy.value = true
	try {
		await invoke('p2p_set_activity_sharing', { enabled: value === 'yes' })
		shareActivity.value = value
	} catch {
		// Se conserva el valor anterior.
	} finally {
		activityBusy.value = false
	}
}
</script>

<template>
	<div
		v-if="!loggedIn"
		class="flex flex-col gap-2 rounded-2xl bg-surface-3 p-4 ring-1 ring-inset ring-surface-5"
	>
		<h3 class="m-0 text-lg font-bold text-contrast">Crea tu cuenta para gestionar tu privacidad</h3>
		<p class="m-0 text-secondary">
			Las opciones sociales pertenecen a tu cuenta de Strata. Créala en la pestaña
			<strong class="text-contrast">Perfil</strong>.
		</p>
	</div>

	<div v-else-if="profile" class="flex flex-col gap-8">
		<Admonition
			v-if="error"
			type="critical"
			:body="error"
			dismissible
			@dismiss="account.clearError()"
		/>

		<section class="flex flex-col gap-6">
			<div class="flex flex-col gap-2.5">
				<h2 class="m-0 text-lg font-semibold text-contrast">Solicitudes de amistad</h2>
				<Chips
					:model-value="profile.settings.friend_privacy"
					:items="friendOptions"
					:format-label="(value: FriendPrivacy) => friendLabels[value]"
					:disabled-items="busy ? friendOptions : undefined"
					:capitalize="false"
					aria-label="Solicitudes de amistad"
					@update:model-value="
						(value: FriendPrivacy | null) =>
							value && account.update({ settings: { friend_privacy: value } })
					"
				/>
				<p class="m-0 text-secondary">
					Controla quién puede enviarte solicitudes de amistad en Strata.
				</p>
			</div>

			<div class="flex flex-col gap-2.5">
				<h2 class="m-0 text-lg font-semibold text-contrast">Invitaciones a partidas</h2>
				<Chips
					:model-value="profile.settings.invite_privacy"
					:items="inviteOptions"
					:format-label="(value: InvitePrivacy) => inviteLabels[value]"
					:disabled-items="busy ? inviteOptions : undefined"
					:capitalize="false"
					aria-label="Invitaciones a partidas"
					@update:model-value="
						(value: InvitePrivacy | null) =>
							value && account.update({ settings: { invite_privacy: value } })
					"
				/>
				<p class="m-0 text-secondary">
					Controla quién puede invitarte a mundos o servidores compartidos. «Nadie» desactiva las
					invitaciones, aunque seáis amigos.
				</p>
			</div>

			<div class="flex flex-col gap-2.5">
				<h2 class="m-0 text-lg font-semibold text-contrast">Aparecer en el buscador</h2>
				<Chips
					:model-value="searchable"
					:items="[...searchableOptions]"
					:format-label="(value: 'yes' | 'no') => searchableLabels[value]"
					:disabled-items="busy ? [...searchableOptions] : undefined"
					:capitalize="false"
					aria-label="Aparecer en el buscador"
					@update:model-value="
						(value: 'yes' | 'no' | null) =>
							value && account.update({ settings: { searchable: value === 'yes' } })
					"
				/>
				<p class="m-0 text-secondary">
					Si lo desactivas, nadie te encontrará buscando por nombre; solo podrán añadirte quienes
					sepan tu nombre de usuario exacto.
				</p>
			</div>

			<div class="flex flex-col gap-2.5">
				<h2 class="m-0 text-lg font-semibold text-contrast">Mostrar a qué juego</h2>
				<Chips
					:model-value="shareActivity"
					:items="[...searchableOptions]"
					:format-label="(value: 'yes' | 'no') => searchableLabels[value]"
					:disabled-items="activityBusy ? [...searchableOptions] : undefined"
					:capitalize="false"
					aria-label="Mostrar a qué juego"
					@update:model-value="setShareActivity"
				/>
				<p class="m-0 text-secondary">
					Tus amigos ven «Jugando a …» con el nombre de la instancia que tienes abierta. Si lo
					desactivas, solo verán que estás conectado.
				</p>
			</div>
		</section>

		<section class="flex flex-col gap-3">
			<div class="flex flex-col gap-1">
				<h2 class="m-0 text-lg font-semibold text-contrast">Usuarios bloqueados</h2>
				<p class="m-0 text-secondary">
					Estos son los usuarios que bloqueaste en Strata. Ellos no pueden:
				</p>
				<ul class="m-0 flex list-disc flex-col gap-1 pl-5 text-secondary">
					<li>Enviarte solicitudes de amistad</li>
					<li>Aparecer en tus búsquedas ni encontrarte</li>
					<li>Invitarte a partidas</li>
				</ul>
			</div>

			<div class="overflow-hidden rounded-2xl border border-solid border-surface-4">
				<div
					class="flex items-center justify-between bg-surface-3 px-4 py-2.5 font-semibold text-contrast"
				>
					<span>Usuario</span><span>Acciones</span>
				</div>
				<div
					v-if="!social.blocked.length"
					class="flex h-32 items-center justify-center px-4 text-center text-secondary"
				>
					No bloqueaste a nadie.
				</div>
				<div
					v-for="user in social.blocked"
					:key="user.id"
					class="flex items-center justify-between gap-3 border-0 border-t border-solid border-surface-4 px-4 py-2.5"
				>
					<span class="min-w-0 truncate font-semibold text-contrast">{{ user.username }}</span>
					<Button
						type="outlined"
						size="sm"
						:disabled="busy"
						@click="account.friendAction('unblock', user.username)"
					>
						Desbloquear
					</Button>
				</div>
			</div>
		</section>
	</div>
</template>
