<script setup lang="ts">
import {
	CoffeeIcon,
	HeartHandshakeIcon,
	LanguagesIcon,
	LightBulbIcon,
	MicrochipIcon,
	PaintbrushIcon,
	RefreshCwIcon,
	Settings2Icon,
	ShieldIcon,
	ToggleRightIcon,
	UserIcon,
	VideoIcon,
} from '@modrinth/assets'
import {
	commonMessages,
	commonSettingsMessages,
	defineMessage,
	defineMessages,
	injectNotificationManager,
	Button,
	ProgressBar,
	TabbedModal,
	UnsavedChangesPopup,
	useVIntl,
} from '@modrinth/ui'
import { useMutation, useQuery, useQueryClient } from '@tanstack/vue-query'
import { getVersion } from '@tauri-apps/api/app'
import { platform as getOsPlatform, version as getOsVersion } from '@tauri-apps/plugin-os'
import { computed, provide, ref } from 'vue'

import PrivacySettings from '@/components/ui/settings/account/PrivacySettings.vue'
import StrataBrand from '@/components/ui/StrataBrand.vue'
import ProfileSettings from '@/components/ui/settings/account/ProfileSettings.vue'
import SocialSettings from '@/components/ui/settings/account/SocialSettings.vue'
import AppearanceSettings from '@/components/ui/settings/display/AppearanceSettings.vue'
import BehaviorSettings from '@/components/ui/settings/display/BehaviorSettings.vue'
import FeatureFlagSettings from '@/components/ui/settings/display/FeatureFlagSettings.vue'
import FeaturesSettings from '@/components/ui/settings/display/FeaturesSettings.vue'
import LanguageSettings from '@/components/ui/settings/display/LanguageSettings.vue'
import RecordingSettings from '@/components/ui/settings/display/RecordingSettings.vue'
import InstancesSyncedSettings from '@/components/ui/settings/instances/instances-synced-settings/index.vue'
import JavaSettings from '@/components/ui/settings/instances/JavaSettings.vue'
import ResourceManagementSettings from '@/components/ui/settings/instances/ResourceManagementSettings.vue'
import { useAppSettings } from '@/composables/use-app-settings.ts'
import { appSettingsKeys, appSettingsQueryOptions, set } from '@/helpers/settings.ts'
import {
	appSettingsModalContextKey,
	type UnsavedChangesController,
} from '@/providers/app-settings-modal'
import {
	appUpdateState,
	checkForAppUpdate,
	installAvailableAppUpdate,
} from '@/providers/app-update'
import { injectAppUpdateDownloadProgress } from '@/providers/download-progress.ts'

// TODO: Apply COMPONENT_STRUCTURE.md here and extract out common setting option components
const appSettings = useAppSettings()

const { formatMessage } = useVIntl()
const { handleError } = injectNotificationManager()
const queryClient = useQueryClient()

const devModeCounter = ref(0)

const developerModeEnabled = defineMessage({
	id: 'app.settings.developer-mode-enabled',
	defaultMessage: 'Developer mode enabled.',
})

const tabCategories = defineMessages({
	display: {
		id: 'settings.sidebar.label.display',
		defaultMessage: 'Display',
	},
	account: {
		id: 'settings.sidebar.label.account',
		defaultMessage: 'Account',
	},
	instances: {
		id: 'app.settings.sidebar.label.instances',
		defaultMessage: 'Instances',
	},
})

const tabs = [
	{
		name: defineMessage({
			id: 'app.settings.tabs.appearance',
			defaultMessage: 'Appearance',
		}),
		category: tabCategories.display,
		icon: PaintbrushIcon,
		content: AppearanceSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.features',
			defaultMessage: 'Features',
		}),
		category: tabCategories.display,
		icon: LightBulbIcon,
		content: FeaturesSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.recording',
			defaultMessage: 'Grabación',
		}),
		category: tabCategories.display,
		icon: VideoIcon,
		content: RecordingSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.behavior',
			defaultMessage: 'Behavior',
		}),
		category: tabCategories.display,
		icon: Settings2Icon,
		content: BehaviorSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.language',
			defaultMessage: 'Language',
		}),
		category: tabCategories.display,
		icon: LanguagesIcon,
		content: LanguageSettings,
		badge: commonMessages.beta,
	},
	{
		name: commonSettingsMessages.featureFlags,
		category: tabCategories.display,
		icon: ToggleRightIcon,
		content: FeatureFlagSettings,
		developerOnly: true,
	},
	{
		name: commonSettingsMessages.profile,
		category: tabCategories.account,
		icon: UserIcon,
		content: ProfileSettings,
	},
	{
		name: commonSettingsMessages.social,
		category: tabCategories.account,
		icon: HeartHandshakeIcon,
		content: SocialSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.privacy',
			defaultMessage: 'Privacy',
		}),
		category: tabCategories.account,
		icon: ShieldIcon,
		content: PrivacySettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.synced-options',
			defaultMessage: 'Synced settings',
		}),
		category: tabCategories.instances,
		icon: RefreshCwIcon,
		content: InstancesSyncedSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.java-installations',
			defaultMessage: 'Java installations',
		}),
		category: tabCategories.instances,
		icon: CoffeeIcon,
		content: JavaSettings,
	},
	{
		name: defineMessage({
			id: 'app.settings.tabs.resource-management',
			defaultMessage: 'Resource management',
		}),
		category: tabCategories.instances,
		icon: MicrochipIcon,
		content: ResourceManagementSettings,
	},
]

const availableTabs = computed(() =>
	tabs.filter((tab) => !tab.developerOnly || appSettings.devMode),
)

const modal = ref<InstanceType<typeof TabbedModal> | null>(null)
const unsavedChangesPopup = ref<{ nudge: () => void } | null>(null)
const unsavedChangesController = ref<UnsavedChangesController | null>(null)
const emptyUnsavedChangesState: Record<string, unknown> = {}
const originalUnsavedChangesState = computed(
	() => unsavedChangesController.value?.getOriginal() ?? emptyUnsavedChangesState,
)
const modifiedUnsavedChangesState = computed(
	() => unsavedChangesController.value?.getModified() ?? emptyUnsavedChangesState,
)
const savingUnsavedChanges = computed(() => unsavedChangesController.value?.isSaving() ?? false)
const hasUnsavedChanges = computed(
	() =>
		(unsavedChangesController.value?.hasChanges() ?? false) ||
		(unsavedChangesController.value?.isSaving() ?? false),
)

function canLeaveCurrentTab(): boolean {
	if (
		!unsavedChangesController.value?.hasChanges() &&
		!unsavedChangesController.value?.isSaving()
	) {
		return true
	}
	unsavedChangesPopup.value?.nudge()
	return false
}

function close(): boolean {
	return modal.value?.hide() ?? false
}

function registerUnsavedChangesController(controller: UnsavedChangesController | null): void {
	unsavedChangesController.value = controller
}

provide(appSettingsModalContextKey, {
	close,
	registerUnsavedChangesController,
})

function resetUnsavedChanges(): void {
	unsavedChangesController.value?.reset()
}

function saveUnsavedChanges(): void {
	void unsavedChangesController.value?.save()
}

function show() {
	modal.value?.show()
}

function showProfile(): void {
	const profileTabIndex = availableTabs.value.findIndex((tab) => tab.content === ProfileSettings)
	if (profileTabIndex >= 0) {
		modal.value?.setTab(profileTabIndex)
	}
	modal.value?.show()
}

function showFeatureFlags(): void {
	const featureFlagsTabIndex = availableTabs.value.findIndex(
		(tab) => tab.content === FeatureFlagSettings,
	)
	if (featureFlagsTabIndex >= 0) {
		modal.value?.setTab(featureFlagsTabIndex)
	}
	modal.value?.show()
}

function showSyncedOptions(): void {
	const syncedOptionsTabIndex = availableTabs.value.findIndex(
		(tab) => tab.content === InstancesSyncedSettings,
	)
	if (syncedOptionsTabIndex >= 0) {
		modal.value?.setTab(syncedOptionsTabIndex)
	}
	modal.value?.show()
}

function showRecording(): void {
	const recordingTabIndex = availableTabs.value.findIndex((tab) => tab.content === RecordingSettings)
	if (recordingTabIndex >= 0) {
		modal.value?.setTab(recordingTabIndex)
	}
	modal.value?.show()
}

defineExpose({ show, showProfile, showFeatureFlags, showSyncedOptions, showRecording })

const { progress, version: downloadingVersion } = injectAppUpdateDownloadProgress()

const { data: appInfo } = useQuery({
	queryKey: ['app-info'],
	queryFn: async () => ({
		version: await getVersion(),
		osPlatform: getOsPlatform(),
		osVersion: getOsVersion(),
	}),
	staleTime: Infinity,
})

const developerModeMutation = useMutation({
	mutationKey: appSettingsKeys.update,
	scope: { id: 'app-settings' },
	mutationFn: async (enabled: boolean) => {
		const settings = await queryClient.fetchQuery(appSettingsQueryOptions())
		const nextSettings = { ...settings, developer_mode: enabled }
		await set(nextSettings)
		return nextSettings
	},
	onMutate: () => queryClient.cancelQueries({ queryKey: appSettingsKeys.all }),
	onSuccess: (settings) => {
		const selectedTab = modal.value ? availableTabs.value[modal.value.selectedTab] : undefined

		queryClient.setQueryData(appSettingsKeys.all, settings)
		appSettings.devMode = settings.developer_mode

		if (modal.value) {
			const selectedTabIndex = selectedTab ? availableTabs.value.indexOf(selectedTab) : -1
			modal.value.setTab(selectedTabIndex >= 0 ? selectedTabIndex : 0)
		}
	},
	onError: handleError,
	onSettled: () => queryClient.invalidateQueries({ queryKey: appSettingsKeys.all }),
})

function devModeCount() {
	if (developerModeMutation.isPending.value) return
	devModeCounter.value++
	if (devModeCounter.value > 5) {
		devModeCounter.value = 0
		developerModeMutation.mutate(!appSettings.devMode)
	}
}

const { checking, lastCheckResult, availableUpdate, finishedDownloading, downloading } =
	appUpdateState

const updateStatus = computed(() => {
	if (checking.value) return 'Buscando actualizaciones…'
	if (availableUpdate.value) {
		const version = availableUpdate.value.version
		if (finishedDownloading.value) return `La versión ${version} está lista para instalar.`
		if (downloading.value) return `Descargando la versión ${version}…`
		return `Hay una versión nueva (${version}).`
	}
	if (lastCheckResult.value === 'up-to-date') return 'Tienes la última versión.'
	if (lastCheckResult.value === 'error')
		return 'No se pudo comprobar. Revisa tu conexión e inténtalo de nuevo.'
	if (lastCheckResult.value === 'disabled')
		return 'Las actualizaciones están desactivadas en esta versión.'
	return ''
})

const messages = defineMessages({
	downloading: {
		id: 'app.settings.downloading',
		defaultMessage: 'Downloading v{version}',
	},
	appVersion: {
		id: 'app.settings.app-version',
		defaultMessage: 'Strata {version}',
	},
	macos: {
		id: 'app.settings.operating-system.macos',
		defaultMessage: 'macOS',
	},
	developerModeButtonLabel: {
		id: 'app.settings.developer-mode-button.label',
		defaultMessage: 'Toggle developer mode',
	},
})
</script>
<template>
	<TabbedModal
		ref="modal"
		:tabs="availableTabs"
		:width="'min(928px, calc(95vw - 10rem))'"
		:before-hide="canLeaveCurrentTab"
		:before-tab-change="canLeaveCurrentTab"
		:floating-action-bar-shown="hasUnsavedChanges"
	>
		<template #title>
			<span class="text-2xl font-semibold text-contrast">
				{{ formatMessage(commonMessages.settingsLabel) }}
			</span>
		</template>
		<template #floating-action-bar>
			<UnsavedChangesPopup
				ref="unsavedChangesPopup"
				:original="originalUnsavedChangesState"
				:modified="modifiedUnsavedChangesState"
				:saving="savingUnsavedChanges"
				inline
				@reset="resetUnsavedChanges"
				@save="saveUnsavedChanges"
			/>
		</template>
		<template #footer>
			<div class="mt-auto text-secondary text-sm">
				<div class="mb-3">
					<template v-if="progress > 0 && progress < 1">
						<p class="m-0 mb-2">
							{{ formatMessage(messages.downloading, { version: downloadingVersion }) }}
						</p>
						<ProgressBar :progress="progress" />
					</template>
				</div>
				<div class="mb-3 flex flex-col gap-2">
						<p v-if="updateStatus" class="m-0" aria-live="polite">{{ updateStatus }}</p>
						<div class="flex flex-wrap gap-2">
							<Button
								v-if="finishedDownloading && availableUpdate"
								color="brand"
								@click="installAvailableAppUpdate"
							>
								<RefreshCwIcon /> Reiniciar y actualizar
							</Button>
							<Button v-else :disabled="checking" @click="checkForAppUpdate">
								<RefreshCwIcon :class="{ 'animate-spin': checking }" /> Buscar actualizaciones
							</Button>
						</div>
					</div>
					<p v-if="appSettings.devMode" class="text-brand font-semibold m-0 mb-2">
					{{ formatMessage(developerModeEnabled) }}
				</p>
				<div class="flex items-center gap-3">
					<button
						:aria-label="formatMessage(messages.developerModeButtonLabel)"
						:disabled="developerModeMutation.isPending.value"
						class="p-0 m-0 bg-transparent border-none cursor-pointer button-animation"
						:class="{
							'text-brand': appSettings.devMode,
							'text-secondary': !appSettings.devMode,
						}"
						@click="devModeCount"
					>
						<StrataBrand :wordmark="false" class="text-2xl" />
					</button>
					<div v-if="appInfo" class="max-w-[200px]">
						<p class="m-0">
							{{ formatMessage(messages.appVersion, { version: appInfo.version }) }}
						</p>
						<p class="m-0">
							<span v-if="appInfo.osPlatform === 'macos'">{{ formatMessage(messages.macos) }}</span>
							<span v-else class="capitalize">{{ appInfo.osPlatform }}</span>
							{{ appInfo.osVersion }}
						</p>
					</div>
				</div>
			</div>
		</template>
	</TabbedModal>
</template>
