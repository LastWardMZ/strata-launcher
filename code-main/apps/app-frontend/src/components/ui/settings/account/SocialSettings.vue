<script setup lang="ts">
import type { Labrinth } from '@modrinth/api-client'
import { AccountSocialSettings, injectAuth, injectModrinthClient } from '@modrinth/ui'
import { inject, onBeforeUnmount, onMounted, ref } from 'vue'

import ModrinthConnection from '@/components/ui/settings/account/ModrinthConnection.vue'
import { appSettingsModalContextKey } from '@/providers/app-settings-modal'

const auth = injectAuth()
const client = injectModrinthClient()
const modal = inject(appSettingsModalContextKey, null)
const social = ref<InstanceType<typeof AccountSocialSettings> | null>(null)

function getBlockedUsers(): Promise<Labrinth.BlockedUsers.v3.BlockedUserId[]> {
	return client.labrinth.blocked_users_v3.list()
}
function getUsers(userIds: string[]): Promise<Labrinth.Users.v2.User[]> {
	return client.labrinth.users_v2.getMultiple(userIds)
}
function unblockUser(userId: string): Promise<void> {
	return client.labrinth.blocked_users_v3.unblock(userId)
}

onMounted(() =>
	modal?.registerUnsavedChangesController({
		hasChanges: () => social.value?.hasChanges ?? false,
		getOriginal: () => social.value?.originalState ?? {},
		getModified: () => social.value?.modifiedState ?? {},
		isSaving: () => social.value?.saving ?? false,
		reset: () => social.value?.reset(),
		save: () => social.value?.save(),
	}),
)
onBeforeUnmount(() => modal?.registerUnsavedChangesController(null))
</script>

<template>
	<!-- Sin cuenta de Modrinth conectada se muestra la conexión, en vez del aviso genérico de inicio de sesión. -->
	<ModrinthConnection v-if="!auth.user.value" />
	<AccountSocialSettings
		v-else
		ref="social"
		:get-blocked-users="getBlockedUsers"
		:get-users="getUsers"
		:unblock-user="unblockUser"
	/>
</template>
