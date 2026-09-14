const net = require('node:net');
const { once } = require('node:events');

// 只通过 Docker 的标准输入输出传输，不在宿主或容器开放额外 TCP 端口。
function transport(input, output, connect, firstId) {
  const sockets = new Map();
  let nextId = firstId, buffered = Buffer.alloc(0), stopped = false;
  const listeners = new Set();
  function send(kind, id, payload = Buffer.alloc(0)) {
    if (stopped) return Promise.resolve();
    const header = Buffer.alloc(9);
    header.writeUInt32BE(payload.length, 0); header.writeUInt32BE(id, 4); header[8] = kind;
    if (!output.write(Buffer.concat([header, payload]))) return once(output, 'drain');
    return Promise.resolve();
  }
  function attach(id, socket) {
    if (sockets.size >= 64) { socket.destroy(); void send(4, id); return; }
    sockets.set(id, socket);
    socket.on('data', chunk => {
      socket.pause();
      const chunks = [];
      for (let offset = 0; offset < chunk.length; offset += 65536) chunks.push(send(2, id, chunk.subarray(offset, offset + 65536)));
      Promise.all(chunks).then(() => socket.resume(), () => socket.destroy());
    });
    socket.on('end', () => void send(3, id));
    socket.on('error', () => socket.destroy());
    socket.on('close', () => { if (sockets.delete(id)) void send(4, id); });
  }
  async function receive(kind, id, body) {
    if (kind === 0) { for (const listener of listeners) listener(JSON.parse(body)); return; }
    if (kind === 1) { attach(id, connect(body.toString())); return; }
    const socket = sockets.get(id);
    if (!socket) return;
    if (kind === 2) {
      if (socket.writableLength > 8 * 1024 * 1024) { socket.destroy(); return; }
      socket.write(body);
    } else if (kind === 3) socket.end();
    else if (kind === 4) { sockets.delete(id); socket.destroy(); }
    else throw new Error('开发容器传输帧无效');
  }
  input.on('data', chunk => {
    buffered = Buffer.concat([buffered, chunk]);
    while (buffered.length >= 9) {
      const length = buffered.readUInt32BE(0);
      if (length > 1024 * 1024) { input.destroy(new Error('开发容器传输超过配额')); return; }
      if (buffered.length < length + 9) break;
      const id = buffered.readUInt32BE(4), kind = buffered[8], body = buffered.subarray(9, length + 9);
      buffered = buffered.subarray(length + 9);
      void receive(kind, id, body).catch(error => input.destroy(error));
    }
  });
  output.on('error', () => close());
  function close() { stopped = true; for (const socket of sockets.values()) socket.destroy(); sockets.clear(); }
  input.on('end', close);
  return {
    open(socket, destination) { const id = nextId; nextId += 2; void send(1, id, Buffer.from(destination)); attach(id, socket); },
    control(value) { return send(0, 0, Buffer.from(JSON.stringify(value))); },
    onControl(listener) { listeners.add(listener); }, close,
  };
}
module.exports = { transport };
