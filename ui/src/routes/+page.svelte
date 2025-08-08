<script lang="ts">
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<h1>Welcome to Throttle</h1>
<p>Version: {data.version} </p>
<p>Repository: <a href="https://github.com/pacman82/throttle">https://github.com/pacman82/throttle</a></p>
<p>Python client on PyPi: <a href="https://pypi.org/project/throttle-client/">https://pypi.org/project/throttle-client/</a></p>
<p>Rust client on crates.io: <a href="https://crates.io/crates/throttle-client">https://crates.io/crates/throttle-client</a></p>