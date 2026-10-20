// oxlint-disable no-unused-expressions
// Adapted from upstream/site_api.js (Apache-2.0).
// Current builds hide API object keys behind proxies and bind their methods.
// Recover the exported object and method names from the same loaded module.
(select) => async () => {
    const deadline = Date.now() + 10000;
    const knownGets = new Set();
    const isGet = value => knownGets.has(value) || (typeof value === "function" &&
        String(value).includes(".get(") && String(value).endsWith(".data"));
    const loaded = new Set();
    const members = [];
    while (Date.now() < deadline) {
        const scripts = performance.getEntriesByType("resource").map(entry => entry.name)
            .filter(url => /\/assets\/.*\.js$/.test(url));
        const entry = scripts.find(url => /\/main-[^/]+\.js$/.test(url));
        if (entry) {
            const directory = entry.slice(0, entry.lastIndexOf("/") + 1);
            for (const url of new Set(scripts.filter(url => url.startsWith(directory)))) {
                if (loaded.has(url)) continue;
                const module = await import(url);
                loaded.add(url);
                let opaque = false;
                for (const exported of Object.values(module)) {
                    if (!exported || typeof exported !== "object") continue;
                    const entries = Object.entries(exported);
                    if (entries.length === 0) opaque = true;
                    for (const [key, value] of entries) {
                        members.push({ owner: exported, key, value });
                        if (value && typeof value === "object") {
                            for (const [nestedKey, nested] of Object.entries(value)) {
                                members.push({ owner: value, key: nestedKey, value: nested });
                            }
                        }
                    }
                }
                if (opaque) {
                    const response = await fetch(url);
                    if (!response.ok) throw new Error(`Could not read Comix's API module: HTTP ${response.status}`);
                    const source = await response.text();
                    if (source.length > 1024 * 1024) throw new Error("Comix's API module is too large");
                    const exports = new Map(
                        (source.match(/export\{([^}]+)\}/)?.[1] || "").split(",")
                            .map(pair => pair.trim().split(/\s+as\s+/))
                            .map(([local, name]) => [local, name || local])
                    );
                    const objects = [...source.matchAll(/(?:var|let|const)\s+([\w$]+)=[\w$]+\(\{/g)];
                    for (let i = 0; i < objects.length; i++) {
                        const object = objects[i];
                        const owner = module[exports.get(object[1])];
                        if (!owner || typeof owner !== "object") continue;
                        const body = source.slice(object.index, objects[i + 1]?.index ?? source.length);
                        const getKey = body.match(
                            /^[\w\s$]+=[\w$]+\(\{([\w$]+):async\([\w$]+,[\w$]+\)=>\(await [\w$]+\.get\(/
                        )?.[1];
                        if (getKey && typeof owner[getKey] === "function") {
                            const value = owner[getKey];
                            knownGets.add(value);
                            members.push({ owner, key: getKey, value });
                        }
                    }
                }
                const found = select(members, isGet);
                if (found) return found;
            }
        }
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    return null;
}
