// Adapted from Keiyoushi Comix 1.6.54 (Apache-2.0); see ../upstream/NOTICE.
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

async function chapterApi(siteApi) {
    const api = await siteApi((members, isGet) => {
        const method = members.map(member => member.value).find(value => {
            const source = typeof value === "function" && String(value);
            return source &&
                /^(?:async [\w$]+ ?|async ?|[\w$]+ ?)?\([\w$]+,[\w$]+=\{\}\)(?:=>|\{)/.test(source) &&
                source.includes(".items.map(") && source.includes(".meta") &&
                source.includes("{params:") && !/\.items\.map\([\w$]+=>/.test(source);
        });
        const target = members.find(member => isGet(member.value));
        if (!method || !target) return null;
        const records = [];
        const original = target.value;
        target.owner[target.key] = async (url, config) => {
            const result = await original.call(target.owner, url, config);
            if (/^\/manga\/[^/]+\/chapters$/.test(String(url))) {
                records.push({ params: config?.params, result });
            }
            return result;
        };
        return { method, records };
    })();
    if (!api) throw new Error("Could not find Comix's chapters API");
    window.chekChapterApi = api;
    return true;
}

async function chapterTarget() {
    await new Promise(resolve => setTimeout(resolve, 300));
    const section = document.querySelector(".mpage__chapters");
    if (!section) throw new Error("Comix's chapter section did not load");
    const pager = section.querySelector(".npager");
    const target = pager ? [...pager.querySelectorAll("button")].filter(button => {
        const rect = button.getBoundingClientRect();
        const box = pager.getBoundingClientRect();
        return rect.width > 0 && rect.left >= Math.max(box.left, 0) &&
            rect.right <= Math.min(box.right, innerWidth);
    }).at(-1) : section.querySelector(".useg__btn.is-on");
    if (!target) throw new Error("Comix's chapter section has no input target");
    const rect = target.getBoundingClientRect();
    if (rect.top < 0 || rect.bottom > innerHeight) {
        return { x: Math.floor(innerWidth / 2), y: Math.floor(innerHeight / 2),
            scroll: Math.round(rect.top - innerHeight / 2) };
    }
    const x = Math.floor(rect.left + rect.width / 2);
    const y = Math.floor(rect.top + rect.height / 2);
    if (!target.contains(document.elementFromPoint(x, y))) {
        throw new Error("Comix's chapter controls are obscured");
    }
    window.chekChapterTapped = false;
    section.addEventListener("click", event => {
        if (!event.isTrusted) return;
        window.chekChapterTapped = target.contains(event.target);
        event.preventDefault();
        event.stopPropagation();
    }, { capture: true, once: true });
    return { x, y, scroll: 0 };
}

async function chapterPage(id, page) {
    if (!window.chekChapterTapped) throw new Error("The chapter section did not receive native input");
    const api = window.chekChapterApi;
    api.records.length = 0;
    await api.method(id, { page, limit: 20, order: { created_at: "desc" } });
    const record = api.records.find(record => record.params?.page === page &&
        record.params?.order?.created_at === "desc");
    const result = record?.result;
    if (!Array.isArray(result?.items) || typeof result?.meta?.lastPage !== "number" ||
        typeof result?.meta?.total !== "number" ||
        !result.items.every(item => typeof item.id === "number" && typeof item.number === "number" &&
            typeof item.url === "string" && item.url.includes(`/title/${id}-`))) {
        throw new Error(`Comix chapter page ${page} had an unexpected response`);
    }
    return result;
}
