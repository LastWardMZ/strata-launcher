<template>
	<span class="strata-text-logo" :class="{ animate }" aria-label="Strata">
		<svg xmlns="http://www.w3.org/2000/svg" viewBox="80 64 352 376" aria-hidden="true">
			<path fill="var(--color-brand)" d="M394 168 256 88 118 168V224L304 332 256 360 166 308 118 336 256 416 394 336V280L208 172 256 144 346 196Z"/>
		</svg>
		<span>Strata</span>
	</span>
</template>
<script setup>
defineProps({ animate: { type: Boolean, default: false } })
</script>
<style scoped>
.strata-text-logo { display:inline-flex; align-items:center; gap:.35em; line-height:1; color:inherit; }
svg { width:1.1em; height:1.1em; }
span span { font-weight:750; letter-spacing:-.045em; }
</style>
