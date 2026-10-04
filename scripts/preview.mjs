import { once } from 'node:events';
import { parseArgs } from 'node:util';
import { serve } from 'vitepress';

const { values } = parseArgs({
  options: {
    host: { type: 'string', default: '127.0.0.1' },
    port: { type: 'string', default: '4173' },
  },
});
const port = Number(values.port);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error('--port must be an integer between 1 and 65535.');
}

const app = await serve({ root: 'docs', port });
// VitePress preview currently ignores --host and listens on IPv6 by default.
// Rebind its existing server so WSL forwards an IPv4 listener to Windows too.
await new Promise((resolve, reject) => {
  app.server.close(error => error ? reject(error) : resolve());
});
app.server.listen(port, values.host);
await once(app.server, 'listening');
const host = ['0.0.0.0', '::'].includes(values.host) ? '127.0.0.1'
  : values.host.includes(':') ? `[${values.host}]` : values.host;
console.log(`Preview ready at http://${host}:${port}/tono/`);

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => app.server.close(() => process.exit(0)));
}
