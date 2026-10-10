<template>
	<svg xmlns="http://www.w3.org/2000/svg" viewBox="80 64 352 376" aria-label="Strata" class="strata-logo">
		<path fill="var(--color-brand)" d="M394 168 256 88 118 168V224L304 332 256 360 166 308 118 336 256 416 394 336V280L208 172 256 144 346 196Z"/>
		<path d="M118 168 256 248V304L118 224ZM256 360 394 280V336L256 416Z" fill="currentColor" opacity=".15"/>
	</svg>
</template>
<style scoped>
.strata-logo { width:5rem; height:5rem; }
@media (prefers-reduced-motion:no-preference) {
	.strata-logo { animation:strata-breathe 2s ease-in-out infinite; }
}
@keyframes strata-breathe { 50% { opacity:.7; transform:scale(.96); } }
</style>
