"""Protocol and ABC definitions for structural typing."""

from abc import ABC, abstractmethod
from typing import Protocol, TypeVar, Generic, Any, Dict, Optional, runtime_checkable

T = TypeVar("T")
K = TypeVar("K")
V = TypeVar("V")


@runtime_checkable
class Serializable(Protocol):
    """Protocol for objects that can be serialized to dict."""

    def to_dict(self) -> Dict[str, Any]:
        """Convert object to dictionary."""
        ...


@runtime_checkable
class Identifiable(Protocol):
    """Protocol for objects with an ID."""

    @property
    def entity_id(self) -> int:
        """Get the entity's unique identifier."""
        ...


class Cacheable(Protocol[T]):
    """Protocol for cacheable objects with generic type."""

    def cache_key(self) -> str:
        """Generate cache key for this object."""
        ...

    def from_cache(self, data: Dict[str, Any]) -> T:
        """Reconstruct object from cached data."""
        ...


class Comparable(Protocol):
    """Protocol for comparable objects."""

    def __lt__(self, other: Any) -> bool:
        ...

    def __eq__(self, other: object) -> bool:
        ...


class Repository(ABC, Generic[T, K]):
    """Abstract base class for repository pattern."""

    @abstractmethod
    def get(self, key: K) -> Optional[T]:
        """Retrieve an entity by key."""
        pass

    @abstractmethod
    def save(self, entity: T) -> None:
        """Save an entity."""
        pass

    @abstractmethod
    def delete(self, key: K) -> bool:
        """Delete an entity by key."""
        pass

    @abstractmethod
    def list_all(self) -> list[T]:
        """List all entities."""
        pass

    def exists(self, key: K) -> bool:
        """Check if entity exists."""
        return self.get(key) is not None


class EventHandler(ABC):
    """Abstract base class for event handlers."""

    @property
    @abstractmethod
    def event_type(self) -> str:
        """Get the type of event this handler processes."""
        pass

    @abstractmethod
    def handle(self, event: Dict[str, Any]) -> None:
        """Handle the event."""
        pass

    def can_handle(self, event: Dict[str, Any]) -> bool:
        """Check if this handler can process the event."""
        return event.get("type") == self.event_type


class Validator(ABC, Generic[T]):
    """Abstract validator for input validation."""

    @abstractmethod
    def validate(self, value: T) -> bool:
        """Validate the value."""
        pass

    @abstractmethod
    def get_errors(self) -> list[str]:
        """Get validation error messages."""
        pass

    def is_valid(self, value: T) -> bool:
        """Convenience method for validation check."""
        return self.validate(value) and len(self.get_errors()) == 0


class Middleware(ABC):
    """Abstract middleware for request processing."""

    @abstractmethod
    async def process(self, request: Any, next_handler: Any) -> Any:
        """Process request and call next handler."""
        pass

    @abstractmethod
    def should_process(self, request: Any) -> bool:
        """Determine if this middleware should process the request."""
        pass
