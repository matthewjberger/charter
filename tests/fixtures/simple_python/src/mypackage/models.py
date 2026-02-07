"""Data models for the application."""

from dataclasses import dataclass, field
from typing import Optional, List, Dict, Any, ClassVar
from enum import Enum, auto
from datetime import datetime


class Status(Enum):
    """User account status enumeration."""
    PENDING = auto()
    ACTIVE = auto()
    SUSPENDED = auto()
    DELETED = auto()


@dataclass
class Config:
    """Application configuration."""
    database_url: str
    api_key: str
    debug: bool = False
    max_connections: int = 100
    allowed_hosts: List[str] = field(default_factory=list)
    metadata: Dict[str, Any] = field(default_factory=dict)

    def validate(self) -> bool:
        """Validate configuration values."""
        if not self.database_url:
            raise ValueError("database_url is required")
        if not self.api_key:
            raise ValueError("api_key is required")
        return True


@dataclass(frozen=True)
class Address:
    """Immutable address value object."""
    street: str
    city: str
    country: str
    postal_code: Optional[str] = None


class Entity:
    """Base class for all domain entities."""

    _registry: ClassVar[Dict[int, "Entity"]] = {}

    def __init__(self, entity_id: int) -> None:
        self.entity_id = entity_id
        self.created_at = datetime.now()
        self.updated_at: Optional[datetime] = None
        Entity._registry[entity_id] = self

    def update(self) -> None:
        """Mark entity as updated."""
        self.updated_at = datetime.now()

    @classmethod
    def get_by_id(cls, entity_id: int) -> Optional["Entity"]:
        """Retrieve entity from registry."""
        return cls._registry.get(entity_id)

    @staticmethod
    def generate_id() -> int:
        """Generate a new unique ID."""
        import random
        return random.randint(1, 1_000_000)


class User(Entity):
    """User domain model with full lifecycle support."""

    def __init__(
        self,
        entity_id: int,
        username: str,
        email: str,
        status: Status = Status.PENDING,
    ) -> None:
        super().__init__(entity_id)
        self.username = username
        self.email = email
        self.status = status
        self._password_hash: Optional[str] = None
        self.addresses: List[Address] = []
        self.preferences: Dict[str, Any] = {}

    @property
    def is_active(self) -> bool:
        """Check if user account is active."""
        return self.status == Status.ACTIVE

    @property
    def display_name(self) -> str:
        """Get user's display name."""
        return self.username.title()

    @display_name.setter
    def display_name(self, value: str) -> None:
        """Set user's display name."""
        self.username = value.lower()

    def set_password(self, password: str) -> None:
        """Set user password with hashing."""
        import hashlib
        self._password_hash = hashlib.sha256(password.encode()).hexdigest()

    def verify_password(self, password: str) -> bool:
        """Verify password against stored hash."""
        import hashlib
        if self._password_hash is None:
            return False
        return self._password_hash == hashlib.sha256(password.encode()).hexdigest()

    def add_address(self, address: Address) -> None:
        """Add an address to the user."""
        self.addresses.append(address)

    def activate(self) -> None:
        """Activate user account."""
        if self.status == Status.DELETED:
            raise ValueError("Cannot activate deleted user")
        self.status = Status.ACTIVE
        self.update()

    def suspend(self, reason: str) -> None:
        """Suspend user account."""
        self.status = Status.SUSPENDED
        self.preferences["suspension_reason"] = reason
        self.update()

    def to_dict(self) -> Dict[str, Any]:
        """Convert user to dictionary representation."""
        return {
            "id": self.entity_id,
            "username": self.username,
            "email": self.email,
            "status": self.status.name,
            "is_active": self.is_active,
            "created_at": self.created_at.isoformat(),
            "addresses": [
                {"street": a.street, "city": a.city, "country": a.country}
                for a in self.addresses
            ],
        }


class Admin(User):
    """Administrator user with elevated privileges."""

    def __init__(
        self,
        entity_id: int,
        username: str,
        email: str,
        permissions: Optional[List[str]] = None,
    ) -> None:
        super().__init__(entity_id, username, email, Status.ACTIVE)
        self.permissions = permissions or ["read", "write", "delete"]
        self.managed_users: List[int] = []

    def has_permission(self, permission: str) -> bool:
        """Check if admin has specific permission."""
        return permission in self.permissions

    def grant_permission(self, permission: str) -> None:
        """Grant a new permission to admin."""
        if permission not in self.permissions:
            self.permissions.append(permission)

    def revoke_permission(self, permission: str) -> None:
        """Revoke a permission from admin."""
        if permission in self.permissions:
            self.permissions.remove(permission)

    def manage_user(self, user_id: int) -> None:
        """Add user to managed users list."""
        if user_id not in self.managed_users:
            self.managed_users.append(user_id)


class AuditLog:
    """Audit log entry for tracking changes."""

    __slots__ = ("timestamp", "actor_id", "action", "target_type", "target_id", "details")

    def __init__(
        self,
        actor_id: int,
        action: str,
        target_type: str,
        target_id: int,
        details: Optional[Dict[str, Any]] = None,
    ) -> None:
        self.timestamp = datetime.now()
        self.actor_id = actor_id
        self.action = action
        self.target_type = target_type
        self.target_id = target_id
        self.details = details or {}
