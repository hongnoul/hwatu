(() => {
  'use strict';

  if (globalThis.__hwatuOPFS || !globalThis.indexedDB || !globalThis.isSecureContext)
    return;
  if (!globalThis.navigator || !navigator.storage)
    return;

  globalThis.__hwatuOPFS = true;

  const DB_NAME = 'dev.hwatu.opfs';
  const DB_VERSION = 1;
  const STORE = 'entries';
  const ROOT = '/';
  const locks = new Set();
  let databasePromise;

  const error = (name, message) => new DOMException(message, name);
  const invalidName = (name) =>
    typeof name !== 'string' || !name || name === '.' || name === '..' || name.includes('/');
  const nameOf = (path) => path === ROOT ? '' : path.slice(path.lastIndexOf('/') + 1);
  const childPath = (parent, name) => parent === ROOT ? `/${name}` : `${parent}/${name}`;
  const prefixOf = (path) => path === ROOT ? ROOT : `${path}/`;

  const request = (value) => new Promise((resolve, reject) => {
    value.onsuccess = () => resolve(value.result);
    value.onerror = () => reject(value.error);
  });

  const database = () => {
    if (!databasePromise) {
      databasePromise = new Promise((resolve, reject) => {
        const open = indexedDB.open(DB_NAME, DB_VERSION);
        open.onupgradeneeded = () => {
          if (!open.result.objectStoreNames.contains(STORE))
            open.result.createObjectStore(STORE);
        };
        open.onsuccess = () => resolve(open.result);
        open.onerror = () => reject(open.error);
        open.onblocked = () => reject(error('InvalidStateError', 'OPFS database is blocked'));
      });
    }
    return databasePromise;
  };

  const readEntry = async (path) => {
    const db = await database();
    return request(db.transaction(STORE).objectStore(STORE).get(path));
  };

  const writeEntry = async (path, entry) => {
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, 'readwrite');
      tx.objectStore(STORE).put(entry, path);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error || error('AbortError', 'OPFS write aborted'));
    });
  };

  const keysBelow = async (path) => {
    const db = await database();
    const prefix = prefixOf(path);
    const range = IDBKeyRange.bound(prefix, `${prefix}\uffff`, false, false);
    return request(db.transaction(STORE).objectStore(STORE).getAllKeys(range));
  };

  const deleteEntries = async (paths) => {
    const db = await database();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, 'readwrite');
      const store = tx.objectStore(STORE);
      for (const path of paths)
        store.delete(path);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error || error('AbortError', 'OPFS deletion aborted'));
    });
  };

  const assertName = (name) => {
    if (invalidName(name))
      throw new TypeError(`Invalid file name: ${String(name)}`);
  };

  const assertDirectory = async (path) => {
    if (path === ROOT)
      return;
    const entry = await readEntry(path);
    if (!entry)
      throw error('NotFoundError', `Directory does not exist: ${path}`);
    if (entry.kind !== 'directory')
      throw error('TypeMismatchError', `Not a directory: ${path}`);
  };

  const entryFor = (kind, blob) => ({
    kind,
    blob: kind === 'file' ? blob || new Blob([]) : undefined,
    lastModified: Date.now(),
  });

  class FileSystemHandle {
    constructor(path, kind) {
      this._path = path;
      this.kind = kind;
      this.name = nameOf(path);
    }

    async isSameEntry(other) {
      return other instanceof FileSystemHandle && other.kind === this.kind && other._path === this._path;
    }

    async queryPermission() { return 'granted'; }
    async requestPermission() { return 'granted'; }
  }

  class WriteState {
    constructor(handle, bytes, type) {
      this.handle = handle;
      this.bytes = bytes;
      this.type = type || '';
      this.position = 0;
      this.closed = false;
      this.queue = Promise.resolve();
    }

    run(operation) {
      const next = this.queue.then(() => {
        if (this.closed)
          throw error('InvalidStateError', 'Writable stream is closed');
        return operation();
      });
      this.queue = next.catch(() => {});
      return next;
    }

    async dataBytes(data) {
      if (typeof data === 'string')
        return new TextEncoder().encode(data);
      if (data instanceof Blob) {
        if (data.type)
          this.type = data.type;
        return new Uint8Array(await data.arrayBuffer());
      }
      if (data instanceof ArrayBuffer)
        return new Uint8Array(data.slice(0));
      if (ArrayBuffer.isView(data))
        return new Uint8Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength));
      throw new TypeError('OPFS write data must be a string, Blob, ArrayBuffer, or typed array');
    }

    write(chunk) {
      return this.run(async () => {
        if (chunk && typeof chunk === 'object' &&
            !(chunk instanceof Blob) && !(chunk instanceof ArrayBuffer) && !ArrayBuffer.isView(chunk) &&
            typeof chunk.type === 'string') {
          if (chunk.type === 'seek')
            return this.seekNow(chunk.position);
          if (chunk.type === 'truncate')
            return this.truncateNow(chunk.size);
          if (chunk.type !== 'write')
            throw new TypeError(`Unknown OPFS write command: ${chunk.type}`);
          if (chunk.position !== undefined)
            this.seekNow(chunk.position);
          chunk = chunk.data;
        }

        const incoming = await this.dataBytes(chunk);
        const end = this.position + incoming.byteLength;
        if (end > this.bytes.byteLength) {
          const grown = new Uint8Array(end);
          grown.set(this.bytes);
          this.bytes = grown;
        }
        this.bytes.set(incoming, this.position);
        this.position = end;
      });
    }

    seekNow(position) {
      position = Number(position);
      if (!Number.isSafeInteger(position) || position < 0)
        throw new TypeError('OPFS seek position must be a non-negative integer');
      this.position = position;
    }

    seek(position) {
      return this.run(() => this.seekNow(position));
    }

    truncateNow(size) {
      size = Number(size);
      if (!Number.isSafeInteger(size) || size < 0)
        throw new TypeError('OPFS truncate size must be a non-negative integer');
      const resized = new Uint8Array(size);
      resized.set(this.bytes.subarray(0, size));
      this.bytes = resized;
      if (this.position > size)
        this.position = size;
    }

    truncate(size) {
      return this.run(() => this.truncateNow(size));
    }

    finish() {
      const next = this.queue.then(async () => {
        if (this.closed)
          return;
        this.closed = true;
        try {
          await writeEntry(this.handle._path, entryFor('file', new Blob([this.bytes], { type: this.type })));
        } finally {
          locks.delete(this.handle._path);
        }
      });
      this.queue = next.catch(() => {});
      return next;
    }

    abort() {
      const next = this.queue.then(() => {
        if (this.closed)
          return;
        this.closed = true;
        locks.delete(this.handle._path);
      });
      this.queue = next.catch(() => {});
      return next;
    }
  }

  class FileSystemWritableFileStream extends WritableStream {
    constructor(state) {
      super({
        write: (chunk) => state.write(chunk),
        close: () => state.finish(),
        abort: () => state.abort(),
      });
      this._state = state;
    }

    write(chunk) { return this._state.write(chunk); }
    seek(position) { return this._state.seek(position); }
    truncate(size) { return this._state.truncate(size); }
    close() { return this._state.finish(); }
    abort() { return this._state.abort(); }
  }

  class FileSystemFileHandle extends FileSystemHandle {
    constructor(path) { super(path, 'file'); }

    async getFile() {
      const entry = await readEntry(this._path);
      if (!entry)
        throw error('NotFoundError', `File does not exist: ${this._path}`);
      if (entry.kind !== 'file')
        throw error('TypeMismatchError', `Not a file: ${this._path}`);
      const blob = entry.blob || new Blob([]);
      return new File([blob], this.name, {
        type: blob.type,
        lastModified: entry.lastModified || Date.now(),
      });
    }

    async createWritable(options = {}) {
      const entry = await readEntry(this._path);
      if (!entry)
        throw error('NotFoundError', `File does not exist: ${this._path}`);
      if (entry.kind !== 'file')
        throw error('TypeMismatchError', `Not a file: ${this._path}`);
      if (locks.has(this._path))
        throw error('NoModificationAllowedError', `File already has an active writer: ${this._path}`);

      locks.add(this._path);
      try {
        const source = options.keepExistingData ? (entry.blob || new Blob([])) : new Blob([]);
        const bytes = new Uint8Array(await source.arrayBuffer());
        return new FileSystemWritableFileStream(new WriteState(this, bytes, source.type));
      } catch (cause) {
        locks.delete(this._path);
        throw cause;
      }
    }
  }

  class FileSystemDirectoryHandle extends FileSystemHandle {
    constructor(path) { super(path, 'directory'); }

    async getFileHandle(name, options = {}) {
      assertName(name);
      await assertDirectory(this._path);
      const path = childPath(this._path, name);
      const existing = await readEntry(path);
      if (existing) {
        if (existing.kind !== 'file')
          throw error('TypeMismatchError', `Not a file: ${path}`);
        return new FileSystemFileHandle(path);
      }
      if (!options.create)
        throw error('NotFoundError', `File does not exist: ${path}`);
      await writeEntry(path, entryFor('file'));
      return new FileSystemFileHandle(path);
    }

    async getDirectoryHandle(name, options = {}) {
      assertName(name);
      await assertDirectory(this._path);
      const path = childPath(this._path, name);
      const existing = await readEntry(path);
      if (existing) {
        if (existing.kind !== 'directory')
          throw error('TypeMismatchError', `Not a directory: ${path}`);
        return new FileSystemDirectoryHandle(path);
      }
      if (!options.create)
        throw error('NotFoundError', `Directory does not exist: ${path}`);
      await writeEntry(path, entryFor('directory'));
      return new FileSystemDirectoryHandle(path);
    }

    async removeEntry(name, options = {}) {
      assertName(name);
      await assertDirectory(this._path);
      const path = childPath(this._path, name);
      const existing = await readEntry(path);
      if (!existing)
        throw error('NotFoundError', `Entry does not exist: ${path}`);

      const descendants = existing.kind === 'directory' ? await keysBelow(path) : [];
      if (descendants.length && !options.recursive)
        throw error('InvalidModificationError', `Directory is not empty: ${path}`);
      if ([...locks].some((locked) => locked === path || locked.startsWith(prefixOf(path))))
        throw error('NoModificationAllowedError', `Entry has an active writer: ${path}`);
      await deleteEntries([path, ...descendants]);
    }

    async *entries() {
      await assertDirectory(this._path);
      const prefix = prefixOf(this._path);
      const paths = await keysBelow(this._path);
      for (const path of paths) {
        const relative = path.slice(prefix.length);
        if (!relative || relative.includes('/'))
          continue;
        const entry = await readEntry(path);
        if (!entry)
          continue;
        yield [relative, entry.kind === 'directory'
          ? new FileSystemDirectoryHandle(path)
          : new FileSystemFileHandle(path)];
      }
    }

    async *keys() {
      for await (const [name] of this.entries())
        yield name;
    }

    async *values() {
      for await (const [, handle] of this.entries())
        yield handle;
    }

    [Symbol.asyncIterator]() { return this.entries(); }

    async resolve(handle) {
      if (!(handle instanceof FileSystemHandle))
        return null;
      if (handle._path === this._path)
        return [];
      const prefix = prefixOf(this._path);
      if (!handle._path.startsWith(prefix))
        return null;
      return handle._path.slice(prefix.length).split('/');
    }
  }

  for (const constructor of [
    FileSystemHandle,
    FileSystemFileHandle,
    FileSystemDirectoryHandle,
    FileSystemWritableFileStream,
  ]) {
    try {
      Object.defineProperty(constructor.prototype, Symbol.toStringTag, {
        configurable: true,
        value: constructor.name,
      });
      Object.defineProperty(globalThis, constructor.name, {
        configurable: true,
        writable: true,
        value: constructor,
      });
    } catch (_) {}
  }

  const getDirectory = async () => {
    await database();
    return new FileSystemDirectoryHandle(ROOT);
  };

  try {
    Object.defineProperty(StorageManager.prototype, 'getDirectory', {
      configurable: true,
      enumerable: true,
      writable: true,
      value: getDirectory,
    });
  } catch (_) {
    try { navigator.storage.getDirectory = getDirectory; } catch (_) {}
  }
})();
