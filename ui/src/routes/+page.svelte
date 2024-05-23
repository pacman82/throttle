<script>
    import { onMount } from 'svelte';

    let backend = window.location.hostname + (window.location.port + 1)

    let version;
    onMount(() => (
        fetch('http://' + backend + '/version')
        .then(response => response.text()
            .then((v) => version = v)
            .catch((e) => console.log(e))
        .catch((e) => console.log(e)))))


</script>

<h1>Throttle</h1>
<p>Throttle version: {version}</p>
