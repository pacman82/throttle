import type { PageLoad } from './$types';

export const load: PageLoad = async ({ params }) => {
    const version = await fetch(`http://localhost:8000/version`);
    return {
        version: version.ok ? await version.text() : { error: 'Failed to fetch version' },
    }
};