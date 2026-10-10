<script setup lang="ts">
import { useTheme } from '@/composables/use-theme'

withDefaults(defineProps<{ wordmark?: boolean; tile?: boolean }>(), { wordmark: true, tile: false })
const theme = useTheme()
</script>

<template>
	<span class="strata-brand" :class="{ 'strata-brand-tile': tile }" aria-label="Strata">
		<img :src="tile ? '/brand/strata-icon.svg' : theme.active === 'light' ? '/brand/strata-mark-light.svg' : '/brand/strata-mark.svg'" alt="" draggable="false" />
		<span v-if="wordmark" class="strata-wordmark">Strata</span>
	</span>
</template>

<style scoped>
.strata-brand {
	display: inline-flex;
	align-items: center;
	gap: .35em;
	flex-shrink: 0;
	font-size: inherit;
	line-height: 1;
	color: var(--color-contrast);
	user-select: none;
}
.strata-brand img {
	width: 1.1em;
	height: 1.1em;
	object-fit: contain;
	flex-shrink: 0;
}
.strata-brand-tile img {
	width: 1em;
	height: 1em;
}
.strata-wordmark {
	font-weight: 750;
	letter-spacing: -.045em;
}
</style>
