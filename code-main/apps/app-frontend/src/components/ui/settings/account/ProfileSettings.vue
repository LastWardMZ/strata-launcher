<script setup lang="ts">
import StrataAccount from './StrataAccount.vue'
import { inject, onMounted, onBeforeUnmount, ref } from 'vue'
import { appSettingsModalContextKey } from '@/providers/app-settings-modal'

const profile = ref<InstanceType<typeof StrataAccount> | null>(null)
const modal = inject(appSettingsModalContextKey, null)
onMounted(() => modal?.registerUnsavedChangesController({
	hasChanges: () => profile.value?.hasChanges ?? false,
	getOriginal: () => profile.value?.originalState ?? {},
	getModified: () => profile.value?.modifiedState ?? {},
	isSaving: () => profile.value?.saving ?? false,
	reset: () => profile.value?.reset(),
	save: () => profile.value?.save(),
}))
onBeforeUnmount(() => modal?.registerUnsavedChangesController(null))
</script>
<template><StrataAccount ref="profile" /></template>
