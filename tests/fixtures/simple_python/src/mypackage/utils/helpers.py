"""Helper utilities and decorators."""

import functools
import time
from typing import TypeVar, Callable, Any, Optional, ParamSpec

P = ParamSpec("P")
R = TypeVar("R")


def retry(
    max_attempts: int = 3,
    delay: float = 1.0,
    exceptions: tuple[type[Exception], ...] = (Exception,),
) -> Callable[[Callable[P, R]], Callable[P, R]]:
    """Decorator to retry function on failure."""

    def decorator(func: Callable[P, R]) -> Callable[P, R]:
        @functools.wraps(func)
        def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
            last_exception: Optional[Exception] = None

            for attempt in range(max_attempts):
                try:
                    return func(*args, **kwargs)
                except exceptions as error:
                    last_exception = error
                    if attempt < max_attempts - 1:
                        time.sleep(delay * (2**attempt))

            raise last_exception or RuntimeError("Retry failed")

        return wrapper

    return decorator


def memoize(func: Callable[P, R]) -> Callable[P, R]:
    """Simple memoization decorator."""
    cache: dict[tuple[Any, ...], R] = {}

    @functools.wraps(func)
    def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
        key = (args, tuple(sorted(kwargs.items())))
        if key not in cache:
            cache[key] = func(*args, **kwargs)
        return cache[key]

    return wrapper


def timing(func: Callable[P, R]) -> Callable[P, R]:
    """Decorator to measure function execution time."""

    @functools.wraps(func)
    def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
        start = time.perf_counter()
        try:
            return func(*args, **kwargs)
        finally:
            elapsed = time.perf_counter() - start
            print(f"{func.__name__} took {elapsed:.4f}s")

    return wrapper


def deprecated(message: str) -> Callable[[Callable[P, R]], Callable[P, R]]:
    """Mark function as deprecated."""

    def decorator(func: Callable[P, R]) -> Callable[P, R]:
        @functools.wraps(func)
        def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
            import warnings
            warnings.warn(
                f"{func.__name__} is deprecated: {message}",
                DeprecationWarning,
                stacklevel=2,
            )
            return func(*args, **kwargs)

        return wrapper

    return decorator


def singleton(cls: type[R]) -> type[R]:
    """Class decorator to implement singleton pattern."""
    instances: dict[type[R], R] = {}

    @functools.wraps(cls, updated=[])
    class SingletonWrapper(cls):  # type: ignore[valid-type,misc]
        def __new__(wrapper_cls: type[R], *args: Any, **kwargs: Any) -> R:
            if cls not in instances:
                instances[cls] = super().__new__(wrapper_cls)
            return instances[cls]

    return SingletonWrapper  # type: ignore[return-value]


class Benchmark:
    """Context manager for benchmarking code blocks."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.start_time: float = 0
        self.end_time: float = 0

    def __enter__(self) -> "Benchmark":
        self.start_time = time.perf_counter()
        return self

    def __exit__(self, *args: Any) -> None:
        self.end_time = time.perf_counter()

    @property
    def elapsed(self) -> float:
        """Get elapsed time in seconds."""
        return self.end_time - self.start_time


def chunk_list(items: list[R], size: int) -> list[list[R]]:
    """Split list into chunks of given size."""
    return [items[i : i + size] for i in range(0, len(items), size)]


def flatten(nested: list[list[R]]) -> list[R]:
    """Flatten nested list."""
    return [item for sublist in nested for item in sublist]


def first_or_none(items: list[R]) -> Optional[R]:
    """Get first item or None if empty."""
    return items[0] if items else None


def unique(items: list[R]) -> list[R]:
    """Remove duplicates while preserving order."""
    seen: set[Any] = set()
    result: list[R] = []
    for item in items:
        if item not in seen:
            seen.add(item)
            result.append(item)
    return result
