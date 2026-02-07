"""Repository implementations for data persistence."""

from typing import Optional, Dict, List, TypeVar, Generic
from mypackage.models import User
from mypackage.protocols import Repository

T = TypeVar("T")
K = TypeVar("K")


class InMemoryRepository(Repository[T, K], Generic[T, K]):
    """Generic in-memory repository implementation."""

    def __init__(self) -> None:
        self._storage: Dict[K, T] = {}

    def get(self, key: K) -> Optional[T]:
        """Retrieve entity by key."""
        return self._storage.get(key)

    def save(self, entity: T) -> None:
        """Save entity to storage."""
        key = self._get_key(entity)
        self._storage[key] = entity

    def delete(self, key: K) -> bool:
        """Delete entity by key."""
        if key in self._storage:
            del self._storage[key]
            return True
        return False

    def list_all(self) -> List[T]:
        """List all entities."""
        return list(self._storage.values())

    def _get_key(self, entity: T) -> K:
        """Extract key from entity. Override in subclass."""
        raise NotImplementedError("Subclass must implement _get_key")

    def clear(self) -> None:
        """Clear all entities from storage."""
        self._storage.clear()

    def count(self) -> int:
        """Count entities in storage."""
        return len(self._storage)


class InMemoryUserRepository(InMemoryRepository[User, int]):
    """In-memory repository for User entities."""

    def _get_key(self, entity: User) -> int:
        """Get user ID as key."""
        return entity.entity_id

    def find_by_username(self, username: str) -> Optional[User]:
        """Find user by username."""
        for user in self._storage.values():
            if user.username == username:
                return user
        return None

    def find_by_email(self, email: str) -> Optional[User]:
        """Find user by email."""
        for user in self._storage.values():
            if user.email == email:
                return user
        return None

    def find_active_users(self) -> List[User]:
        """Find all active users."""
        from mypackage.models import Status
        return [u for u in self._storage.values() if u.status == Status.ACTIVE]


class CachedRepository(Repository[T, K], Generic[T, K]):
    """Repository decorator that adds caching."""

    def __init__(
        self,
        inner: Repository[T, K],
        max_size: int = 1000,
    ) -> None:
        self._inner = inner
        self._cache: Dict[K, T] = {}
        self._max_size = max_size

    def get(self, key: K) -> Optional[T]:
        """Get with cache lookup."""
        if key in self._cache:
            return self._cache[key]

        value = self._inner.get(key)
        if value is not None:
            self._add_to_cache(key, value)
        return value

    def save(self, entity: T) -> None:
        """Save and invalidate cache."""
        self._inner.save(entity)

    def delete(self, key: K) -> bool:
        """Delete and invalidate cache."""
        self._cache.pop(key, None)
        return self._inner.delete(key)

    def list_all(self) -> List[T]:
        """List all from inner repository."""
        return self._inner.list_all()

    def _add_to_cache(self, key: K, value: T) -> None:
        """Add item to cache with size check."""
        if len(self._cache) >= self._max_size:
            oldest_key = next(iter(self._cache))
            del self._cache[oldest_key]
        self._cache[key] = value

    def invalidate(self, key: K) -> None:
        """Invalidate specific cache entry."""
        self._cache.pop(key, None)

    def clear_cache(self) -> None:
        """Clear entire cache."""
        self._cache.clear()
