<script lang="ts">
	import { onMount } from 'svelte';

	//let backend = window.location.host;
    let backend = "localhost:8000";

	let version: string;

    let peers: { id: number; } [] = [];

	const loadData = async () => {
        let request_version = fetch('http://' + backend + '/version');
        let request_peers = fetch('http://' + backend + '/peers/');
		let response_version = await request_version;
		version = await response_version.text();
        let response_peers = await request_peers;
        peers = await response_peers.json();
	};

	onMount(() => loadData().catch((e) => console.log(e)));
</script>

<h1>Throttle</h1>
<p>Throttle version: {version}</p>
<table>
    <th>Peer Id</th>
    {#each peers as peer}
    <tr>
        <td>{ peer.id }</td>
    </tr>
    {/each}
</table>
