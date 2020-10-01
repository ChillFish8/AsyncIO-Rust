import async_rust
import asyncio
import time
import inspect


class AsyncIoCoroutine:
    pass


async def gen():
    print("name")

async def t():
    # loop.call_later(1, gen, "a")
    print(await async_rust.AsyncServerRunner("127.0.0.1:8080", async_rust.OnceFuture))
    # await asyncio.sleep(3)

    fut = asyncio.get_event_loop().create_future()
    fut.__iter__().__next__()

loop = asyncio.get_event_loop()
# loop.set_debug(True)
loop.run_until_complete(t())