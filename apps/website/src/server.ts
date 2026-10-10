import index from "./index.html"

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: Number(process.env.PORT || 18357),
  routes: { "/": index },
  async fetch(request) {
    const pathname = new URL(request.url).pathname
    if (pathname === "/favicon.ico") {
      return new Response(Bun.file(new URL("../public/assets/logos/favicon.ico", import.meta.url)))
    }
    if (!/^\/assets\/(?:fonts|logos|images)\/[a-zA-Z0-9._-]+$/.test(pathname)) {
      return new Response("Not found", { status: 404 })
    }
    const file = Bun.file(new URL(`../public${pathname}`, import.meta.url))
    return (await file.exists()) ? new Response(file) : new Response("Not found", { status: 404 })
  },
  development: { hmr: true, console: true },
})
console.log(`Chek website: ${server.url}`)
