// oxlint-disable no-unused-vars
// Adapted from Keiyoushi Comix (Apache-2.0); see ../upstream/NOTICE.
async function catalogReady() {
    const deadline = Date.now() + 8000;
    while (Date.now() < deadline) {
        const raw = document.querySelector("script#initial-data")?.textContent;
        if (raw) {
            const initial = JSON.parse(raw);
            if (initial && typeof initial.queries === "object" && initial.queries !== null) return true;
        }
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    return false;
}

async function initialData(detail) {
    const deadline = Date.now() + 15000;
    while (Date.now() < deadline) {
        const raw = document.querySelector("script#initial-data")?.textContent;
        if (raw) {
            const queries = JSON.parse(raw).queries || {};
            for (const [key, value] of Object.entries(queries)) {
                const result = value?.result || value;
                if (detail ? key.includes("detail") && result?.hid : Array.isArray(result?.items)) return result;
            }
        }
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    throw new Error(`Comix did not provide catalog data (page: ${document.title.slice(0, 120)})`);
}

function normalizePages(pages, sites) {
    const allows = host => sites.some(site => site.startsWith("*.")
        ? host.endsWith(site.slice(1)) && host.length > site.length - 1
        : host === site);
    return {
        ...pages,
        items: pages.items.map(page => {
            const base = `${(pages.baseUrl || location.origin).replace(/\/+$/, "")}/`;
            const url = new URL(page.url.replace(/^\/+/, ""), base);
            if (!allows(url.hostname)) {
                const label = url.hostname.split(".")[0];
                const alternate = (pages.alt || []).find(domain => allows(`${label}.${domain}`)) ||
                    (/^[a-z0-9]+\.[a-z0-9-]+\.site$/.test(url.hostname) &&
                        url.pathname.startsWith("/hi/") && allows(`${label}.softlifejournal.site`)
                        ? "softlifejournal.site" : null);
                if (!alternate) throw new Error(`Comix image host has no declared mirror: ${url.hostname}`);
                url.hostname = `${label}.${alternate}`;
            }
            return { ...page, url: url.href };
        })
    };
}
