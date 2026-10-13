<script setup lang="ts">
import { CompassIcon, ImportIcon, PlusIcon, ServerStackIcon, UsersIcon } from '@modrinth/assets'
import { Button, defineMessages, IntlFormatted, useVIntl } from '@modrinth/ui'
import { inject, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { appSettingsModalOpenProfileKey } from '@/providers/app-settings-modal'

const showCreationModal = inject<() => void>('showCreationModal')
const showImportModal = inject<() => void>('showImportModal')
const openProfile = inject(appSettingsModalOpenProfileKey, () => {})
const router = useRouter()

const { formatMessage } = useVIntl()

const messages = defineMessages({
	welcomeTitle: {
		id: 'app.welcome-screen.title',
		defaultMessage: 'Welcome to Strata',
	},
	welcomeDescription: {
		id: 'app.welcome-screen.description',
		defaultMessage: 'Create your first instance and start playing in a few clicks.',
	},
	createInstance: {
		id: 'app.welcome-screen.create-instance',
		defaultMessage: 'Create an instance',
	},
	quickCreateHint: {
		id: 'app.welcome-screen.quick-create-hint',
		defaultMessage: 'Press <shortcut>N</shortcut> to quick create an instance',
	},
	importFromLauncher: {
		id: 'app.welcome-screen.import-from-launcher',
		defaultMessage: 'Import from launcher',
	},
	exploreTitle: {
		id: 'app.welcome-screen.explore.title',
		defaultMessage: 'Explore content',
	},
	exploreDescription: {
		id: 'app.welcome-screen.explore.description',
		defaultMessage: 'Find modpacks, mods and shaders to install.',
	},
	serverTitle: {
		id: 'app.welcome-screen.server.title',
		defaultMessage: 'Create a server',
	},
	serverDescription: {
		id: 'app.welcome-screen.server.description',
		defaultMessage: 'Host your own world for you and your friends.',
	},
	friendsTitle: {
		id: 'app.welcome-screen.friends.title',
		defaultMessage: 'Play with friends',
	},
	friendsDescription: {
		id: 'app.welcome-screen.friends.description',
		defaultMessage: 'Create your Strata account to add friends and join their games.',
	},
})

const offline = ref(!navigator.onLine)

function handleOffline() {
	offline.value = true
}

function handleOnline() {
	offline.value = false
}

function handleQuickCreate(event: KeyboardEvent) {
	const target = event.target as HTMLElement | null
	if (
		event.key.toLowerCase() !== 'n' ||
		event.repeat ||
		event.metaKey ||
		event.ctrlKey ||
		event.altKey ||
		target?.isContentEditable ||
		['INPUT', 'TEXTAREA', 'SELECT'].includes(target?.tagName ?? '')
	) {
		return
	}

	if (!offline.value) {
		event.preventDefault()
		showCreationModal?.()
	}
}

onMounted(() => {
	window.addEventListener('offline', handleOffline)
	window.addEventListener('online', handleOnline)
	window.addEventListener('keydown', handleQuickCreate)
})

onUnmounted(() => {
	window.removeEventListener('offline', handleOffline)
	window.removeEventListener('online', handleOnline)
	window.removeEventListener('keydown', handleQuickCreate)
})

const shortcuts = [
	{
		id: 'explore',
		icon: CompassIcon,
		title: messages.exploreTitle,
		description: messages.exploreDescription,
		action: () => router.push('/browse/modpack'),
	},
	{
		id: 'server',
		icon: ServerStackIcon,
		title: messages.serverTitle,
		description: messages.serverDescription,
		action: () => router.push('/servers'),
	},
	{
		id: 'friends',
		icon: UsersIcon,
		title: messages.friendsTitle,
		description: messages.friendsDescription,
		action: () => openProfile(),
	},
]
</script>

<template>
	<div class="flex min-h-full flex-col items-center justify-center gap-10 px-6 py-10">
		<section class="relative isolate flex max-w-xl flex-col items-center gap-6 text-center">
			<div class="welcome-glow pointer-events-none absolute left-1/2 top-0 -z-10" aria-hidden="true" />
			<img
				src="/brand/strata-icon.svg"
				alt=""
				draggable="false"
				class="size-24 rounded-[1.4rem] shadow-lg ring-1 ring-surface-5"
			/>
			<div class="flex flex-col gap-2">
				<h1 class="m-0 text-3xl font-extrabold leading-9 text-contrast">
					{{ formatMessage(messages.welcomeTitle) }}
				</h1>
				<p class="m-0 text-base leading-6 text-secondary">
					{{ formatMessage(messages.welcomeDescription) }}
				</p>
			</div>
			<div class="flex flex-wrap items-center justify-center gap-3">
				<Button
					type="colored"
					color="brand"
					size="lg"
					class="!shadow-none"
					:disabled="offline"
					@click="showCreationModal?.()"
				>
					<PlusIcon />
					{{ formatMessage(messages.createInstance) }}
				</Button>
				<Button size="lg" class="!font-medium" :disabled="offline" @click="showImportModal?.()">
					<ImportIcon />
					{{ formatMessage(messages.importFromLauncher) }}
				</Button>
			</div>
			<span class="flex items-center gap-1 text-sm leading-5 text-secondary">
				<IntlFormatted :message-id="messages.quickCreateHint">
					<template #shortcut="{ children }">
						<kbd
							class="inline-flex h-5 min-w-5 items-center justify-center rounded-md border border-solid border-surface-5 bg-button-bg px-1 text-xs font-normal leading-4 text-primary"
						>
							<component :is="() => children" />
						</kbd>
					</template>
				</IntlFormatted>
			</span>
		</section>

		<ul class="m-0 grid w-full max-w-3xl list-none grid-cols-1 gap-3 p-0 sm:grid-cols-3">
			<li v-for="item in shortcuts" :key="item.id" class="flex">
				<button
					type="button"
					class="welcome-card flex w-full cursor-pointer flex-col items-start gap-3 rounded-2xl border border-solid border-surface-5 bg-surface-3 p-4 text-left text-contrast"
					@click="item.action()"
				>
					<span
						class="flex size-10 items-center justify-center rounded-xl bg-brand-highlight text-brand"
						aria-hidden="true"
					>
						<component :is="item.icon" class="size-5" />
					</span>
					<span class="flex flex-col gap-1">
						<span class="text-base font-semibold leading-5">{{ formatMessage(item.title) }}</span>
						<span class="text-sm leading-5 text-secondary">{{ formatMessage(item.description) }}</span>
					</span>
				</button>
			</li>
		</ul>
	</div>
</template>

<style scoped>
.welcome-glow {
	width: 30rem;
	height: 20rem;
	max-width: 100vw;
	transform: translate(-50%, -3.5rem);
	background: radial-gradient(
		closest-side,
		color-mix(in srgb, var(--color-brand) 26%, transparent),
		transparent
	);
}

.welcome-card {
	transition:
		border-color 0.15s ease,
		background-color 0.15s ease,
		transform 0.15s ease;
}

.welcome-card:hover {
	border-color: var(--color-brand);
	background-color: var(--surface-4, var(--color-button-bg));
}

.welcome-card:focus-visible {
	outline: 2px solid var(--color-brand);
	outline-offset: 2px;
}

@media (prefers-reduced-motion: no-preference) {
	.welcome-card:hover {
		transform: translateY(-2px);
	}
}
</style>
