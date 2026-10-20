// oxlint-disable no-unused-vars
// Adapted from Keiyoushi Comix 1.6.54 (Apache-2.0); see ../upstream/NOTICE.
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
                if (detail ? key.includes("detail") && result?.hid : Array.isArray(result?.items)) {
                    return result;
                }
            }
        }
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    throw new Error(`Comix did not provide catalog data (page: ${document.title.slice(0, 120)}); verification may be required`);
}

async function prepareApi(path, params) {
    const deadline = Date.now() + 15000;
    let stage = "secure module";
    while (Date.now() < deadline) {
        const url = performance.getEntriesByType("resource").map(entry => entry.name)
            .find(url => /\/secure-[^/]+\.js$/.test(url));
        if (url) {
            stage = "codec registry";
            const module = await import(url);
            const response = await fetch(url, { signal: AbortSignal.timeout(10000) });
            if (!response.ok) throw new Error(`Could not read Comix's codec module: HTTP ${response.status}`);
            const source = await response.text();
            if (source.length > 1024 * 1024) throw new Error("Comix's codec module is too large");
            const registries = [...source.matchAll(/\b([\w$]+)=[\w$]+\.\1\|\|/g)].map(match => match[1]);
            let key;
            for (const name of registries) {
                const registry = window[name];
                if (!registry || typeof registry !== "object") continue;
                key = Object.values(registry).find(value => value &&
                    typeof value.kid === "string" && typeof value.material === "function");
                if (key) break;
            }
            if (key) stage = "request installer";
            if (key && typeof module.r === "function") {
                const hooks = [];
                await module.r({ interceptors: {
                    request: { use(handler) { hooks.push(handler); } },
                    response: { use() {} }
                } }, () => {});
                if (hooks.length !== 1) throw new Error("Comix's request preparation changed");
                const request = await hooks[0]({ url: path, method: "get", params, headers: {} });
                const material = key.material();
                if (!ArrayBuffer.isView(material) || material.BYTES_PER_ELEMENT !== 1 || material.length !== 1088) {
                    throw new Error("Comix's codec material changed");
                }
                return { kid: key.kid, material: Array.from(material),
                    query: Object.fromEntries(Object.entries(request.params).map(([key, value]) => [key, String(value)])) };
            }
        }
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    throw new Error(`Comix's protocol bootstrap did not expose its ${stage}`);
}

async function fetchApi(url) {
    const response = await fetch(url, { credentials: "same-origin",
        headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
        signal: AbortSignal.timeout(15000) });
    const body = await response.text();
    if (body.length > 1024 * 1024) throw new Error("Comix API response is too large");
    return { status: response.status, encoding: response.headers.get("x-enc"), body };
}

async function chapterApi(siteApi) {
    const api = await siteApi((members, isGet) => {
        const target = members.find(member => isGet(member.value));
        if (!target) return null;
        const records = [];
        const api = { records, nextRequest: Date.now() + 2500 + Math.random() * 2500 };
        const original = target.value;
        target.owner[target.key] = async (url, config) => {
            if (/^\/manga\/[^/]+\/chapters$/.test(String(url))) {
                api.nextRequest = Date.now() + 2500 + Math.random() * 2500;
            }
            try {
                const result = await original.call(target.owner, url, config);
                if (/^\/manga\/[^/]+\/chapters$/.test(String(url))) {
                    records.push({ params: config?.params, result });
                }
                return result;
            } catch (error) {
                if (/^\/manga\/[^/]+\/chapters$/.test(String(url))) {
                    records.push({ params: config?.params, error });
                }
                throw error;
            }
        };
        return api;
    })();
    if (!api) throw new Error("Could not find Comix's chapters API");
    window.chekChapterApi = api;
    return true;
}

async function chapterTarget(page, field = "date") {
    // Upstream spaces chapter requests 2.5–5 seconds apart in its interceptor.
    // Wait before input so the site's native-input acknowledgement stays fresh.
    const delay = window.chekChapterApi.nextRequest - Date.now();
    if (delay > 0) await new Promise(resolve => setTimeout(resolve, delay));
    await new Promise(resolve => setTimeout(resolve, 300));
    localStorage.setItem("__pu1", JSON.stringify([Date.now()]));
    const section = document.querySelector(".mpage__chapters");
    if (!section) throw new Error("Comix's chapter section did not load");
    const pager = section.querySelector(".npager");
    const target = page === 1
        ? section.querySelector(`button[title^="Sort by ${field}"]`)
        : [...(pager?.querySelectorAll("button") || [])].find(button =>
            !button.disabled && button.textContent.trim() === String(page));
    if (!target) throw new Error("Comix's chapter section has no input target");
    const rect = target.getBoundingClientRect();
    if (rect.top < 0 || rect.bottom > innerHeight) {
        return { x: Math.floor(innerWidth / 2), y: Math.floor(innerHeight / 2),
            scroll: Math.round(rect.top - innerHeight / 2) };
    }
    const x = Math.floor(rect.left + rect.width / 2);
    const y = Math.floor(rect.top + rect.height / 2);
    let under = document.elementFromPoint(x, y);
    if (under?.tagName === "A" && getComputedStyle(under).position === "fixed" &&
        under.getBoundingClientRect().width >= document.documentElement.clientWidth &&
        under.getBoundingClientRect().height >= document.documentElement.clientHeight) {
        under.remove();
        under = document.elementFromPoint(x, y);
    }
    if (!target.contains(under)) {
        throw new Error(`Comix's chapter controls are obscured by ${under?.tagName || "an unknown element"}`);
    }
    window.chekChapterTapped = false;
    document.addEventListener("click", event => {
        if (event.isTrusted) window.chekChapterTapped = target.contains(event.target);
    }, { capture: true, once: true });
    return { x, y, scroll: 0 };
}

async function chapterInputReceived() {
    const deadline = Date.now() + 2000;
    while (!window.chekChapterTapped && Date.now() < deadline) {
        await new Promise(resolve => setTimeout(resolve, 50));
    }
    return window.chekChapterTapped === true;
}

async function chapterPage(id, page, field = "created_at") {
    if (!window.chekChapterTapped) throw new Error("The chapter section did not receive native input");
    const api = window.chekChapterApi;
    let record;
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
        record = api.records.find(record => record.params?.page === page &&
            record.params?.order?.[field] === "desc");
        if (record) break;
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    if (!record) throw new Error(`Comix chapter page ${page} did not finish loading`);
    if (record?.error) {
        const error = record.error;
        const data = error.response?.data;
        const detail = typeof data === "string"
            ? new DOMParser().parseFromString(data, "text/html").title
            : data?.message;
        throw new Error(`Comix chapter page ${page} failed: HTTP ${error.response?.status || "unknown"}${detail ? ` (${String(detail).slice(0, 160)})` : ""}`);
    }
    const result = record?.result;
    if (!Array.isArray(result?.items) || typeof result?.meta?.lastPage !== "number" ||
        typeof result?.meta?.total !== "number" ||
        !result.items.every(item => typeof item.id === "number" && typeof item.number === "number" &&
            typeof item.url === "string" && item.url.includes(`/title/${id}-`))) {
        throw new Error(`Comix chapter page ${page} had an unexpected response`);
    }
    return result;
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
