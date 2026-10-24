// oxlint-disable no-unused-vars
// Read the site's rendered data, without depending on its private API codec.
async function renderedPages() {
    const deadline = Date.now() + 24000;
    const images = new Map();
    while (Date.now() < deadline) {
        const pages = [...document.querySelectorAll('.rpage-page[data-page]')];
        for (const page of pages) {
            const src = page.querySelector('img.rpage-page__img')?.getAttribute('src');
            if (src && src.startsWith('https://')) images.set(Number(page.dataset.page), src);
        }
        if (pages.length && images.size === pages.length &&
            pages.every((page, index) => Number(page.dataset.page) === index + 1)) {
            return { images: pages.map(page => images.get(Number(page.dataset.page))) };
        }
        const missing = pages.find(page => !images.has(Number(page.dataset.page)));
        missing?.scrollIntoView({ behavior: 'instant', block: 'center' });
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('Comix did not render every reader page');
}

function browseHasMore(page) {
    const pager = document.querySelector('nav[aria-label="Pagination"]');
    return [...(pager?.querySelectorAll("button") || [])].some(button =>
        !button.disabled && (Number(button.textContent.trim()) > page ||
            button.getAttribute("aria-label") === "Next page"));
}

async function renderedChapters(id, page) {
    const deadline = Date.now() + 20000;
    while (Date.now() < deadline) {
        const section = document.querySelector(".mpage__chapters");
        const hint = section?.querySelector(".mchap-foot__hint")?.textContent || "";
        const range = /Showing\s+([\d,]+)\s+to\s+([\d,]+)\s+of\s+([\d,]+)\s+items/i.exec(hint);
        const current = Number(section?.querySelector('[aria-current="page"]')?.textContent || "1");
        const descending = section?.querySelector('button[title="Sort by chapter (desc)"]');
        if (range && current === page && descending && !section.querySelector('[role="status"]')) {
            const [start, end, total] = range.slice(1).map(value => Number(value.replaceAll(",", "")));
            const perPage = page === 1 ? end : (start - 1) / (page - 1);
            const items = [...section.querySelectorAll(".mchap-item")].map(row => {
                const anchor = row.querySelector("a.mchap-row__primary");
                const url = anchor?.getAttribute("href") || "";
                const match = /^\/title\/([a-z0-9]+)-[^/]+\/(\d+)-chapter-(\d+(?:\.\d+)?)/i.exec(url);
                if (!match || match[1] !== id.split("-")[0]) throw new Error("Comix chapter row changed");
                const group = row.querySelector(".mchap-row__group");
                return { id: Number(match[2]), url, number: Number(match[3]),
                    group: group ? { name: group.textContent.trim() } : null,
                    isOfficial: group?.classList.contains("is-official") || false };
            });
            if (Number.isInteger(perPage) && perPage > 0 && start > 0 && end <= total &&
                items.length === end - start + 1 && new Set(items.map(item => item.id)).size === items.length) {
                return { items, meta: { page, total, lastPage: Math.ceil(total / perPage) } };
            }
        }
        if (section && /No chapters/i.test(section.textContent) && !section.querySelector('[role="status"]')) {
            if (page !== 1) throw new Error("Comix chapter pagination changed");
            return { items: [], meta: { page: 1, total: 0, lastPage: 1 } };
        }
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error(`Comix chapter page ${page} did not finish rendering`);
}
