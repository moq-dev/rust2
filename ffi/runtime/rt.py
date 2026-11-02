# The runtime every generated Python module includes: the awaitable a future that pends
# is, the queue each event loop reads its wakes from, and `Waker`. Its names start with
# `_` but `Waker`, so no exported name clashes with them.


class _Future(_n.Future):
	"""A Rust future its first poll left pending: awaiting it polls it on the running
	event loop until it finishes. Cancelling the awaiting task frees it, which cancels it in
	Rust and loses nothing, as does dropping it unawaited. Like a coroutine, it is awaited
	once: a second `await` raises `RuntimeError`."""

	__slots__ = ()

	def __await__(self):
		loop = _get_running_loop()
		w = _wakeup(loop)
		token = self.claim()
		try:
			while True:
				r = self.poll(w.queue)
				if r is not _PENDING:
					return r
				parked = w.parked[token] = loop.create_future()
				yield from parked
		finally:
			w.parked.pop(token, None)
			self.close()


class _Wakeup:
	"""One event loop's wakes: Rust queues a token and writes a byte to a socket the loop
	reads, from any thread and without the GIL, and the loop resumes what each names."""

	__slots__ = ("queue", "parked", "wakers", "pump", "__weakref__")

	def __init__(self, loop):
		self.queue = _n.Queue()
		# The future each parked poll waits on, and each `Waker`'s callback, by token.
		self.parked = {}
		self.wakers = {}
		self.pump = None
		fd = self.queue.fileno()
		try:
			loop.add_reader(fd, self.dispatch)
		except NotImplementedError:
			# A proactor loop, Windows' default, reads only through its own calls.
			sock = _socket.fromfd(fd, _socket.AF_INET if _sys.platform == "win32" else _socket.AF_UNIX, _socket.SOCK_STREAM)
			sock.setblocking(False)
			self.pump = loop.create_task(self._pump(loop, sock))

	async def _pump(self, loop, sock):
		with sock:
			while True:
				await loop.sock_recv(sock, 64)
				self.dispatch()

	def dispatch(self):
		loop = None
		for token in self.queue.take():
			if token < 0:
				self.wakers.pop(-token, None)
				continue
			parked = self.parked.pop(token, None)
			if parked is not None and not parked.done():
				parked.set_result(None)
			wake = self.wakers.get(token)
			if wake is not None:
				if loop is None:
					loop = _get_running_loop()
				loop.call_soon(wake)


_wakeups = _weakref.WeakKeyDictionary()


def _wakeup(loop):
	w = _wakeups.get(loop)
	if w is None:
		w = _wakeups[loop] = _Wakeup(loop)
	return w


class Waker:
	"""Wakes a future's poller: Rust may wake it from any thread, any number of times, and
	each wake calls `wake` on the event loop it was made on, never inside Rust's wake. Wakes
	that land before the loop gets to them call it once."""

	__slots__ = ("_node",)

	def __init__(self, wake, loop=None):
		w = _wakeup(_get_running_loop() if loop is None else loop)
		self._node = _n.Node(w.queue)
		w.wakers[self._node.token()] = wake


class _Const:
	"""A constant a class names, made anew each time it is read, as Rust makes it."""

	__slots__ = ("get",)

	def __init__(self, get):
		self.get = get

	def __get__(self, obj, cls=None):
		return self.get()
