"""Service layer with business logic."""

from typing import Optional, List, Dict, Any, AsyncIterator
import asyncio
from dataclasses import dataclass

from mypackage.models import User, Admin, Status, Config
from mypackage.protocols import Repository, Serializable
from mypackage.errors import (
    NotFoundError,
    ValidationError,
    AuthenticationError,
    AuthorizationError,
)


@dataclass
class ServiceResult:
    """Result wrapper for service operations."""
    success: bool
    data: Optional[Any] = None
    error: Optional[str] = None


class UserService:
    """Service for user management operations."""

    def __init__(self, repository: Repository[User, int], config: Config) -> None:
        self.repository = repository
        self.config = config
        self._cache: Dict[int, User] = {}

    def get_user(self, user_id: int) -> User:
        """Get user by ID with caching."""
        if user_id in self._cache:
            return self._cache[user_id]

        user = self.repository.get(user_id)
        if user is None:
            raise NotFoundError("User", user_id)

        self._cache[user_id] = user
        return user

    def create_user(
        self,
        username: str,
        email: str,
        password: str,
    ) -> User:
        """Create a new user with validation."""
        self._validate_username(username)
        self._validate_email(email)
        self._validate_password(password)

        user_id = User.generate_id()
        user = User(user_id, username, email)
        user.set_password(password)

        self.repository.save(user)
        return user

    def _validate_username(self, username: str) -> None:
        """Validate username format."""
        if not username:
            raise ValidationError("Username is required", field="username")
        if len(username) < 3:
            raise ValidationError("Username must be at least 3 characters", field="username")
        if len(username) > 50:
            raise ValidationError("Username must be at most 50 characters", field="username")
        if not username.isalnum():
            raise ValidationError("Username must be alphanumeric", field="username")

    def _validate_email(self, email: str) -> None:
        """Validate email format."""
        if not email:
            raise ValidationError("Email is required", field="email")
        if "@" not in email:
            raise ValidationError("Invalid email format", field="email")
        if len(email) > 255:
            raise ValidationError("Email must be at most 255 characters", field="email")

    def _validate_password(self, password: str) -> None:
        """Validate password strength."""
        errors: List[str] = []
        if len(password) < 8:
            errors.append("Password must be at least 8 characters")
        if not any(c.isupper() for c in password):
            errors.append("Password must contain uppercase letter")
        if not any(c.islower() for c in password):
            errors.append("Password must contain lowercase letter")
        if not any(c.isdigit() for c in password):
            errors.append("Password must contain digit")
        if errors:
            raise ValidationError("Password validation failed", field="password", errors=errors)

    def authenticate(self, username: str, password: str) -> User:
        """Authenticate user by username and password."""
        users = self.repository.list_all()
        for user in users:
            if user.username == username:
                if user.verify_password(password):
                    if not user.is_active:
                        raise AuthenticationError("User account is not active")
                    return user
                raise AuthenticationError("Invalid password")
        raise AuthenticationError("User not found")

    def update_user(
        self,
        user_id: int,
        updates: Dict[str, Any],
        admin: Optional[Admin] = None,
    ) -> User:
        """Update user with optional admin privileges."""
        user = self.get_user(user_id)

        if "status" in updates and admin is None:
            raise AuthorizationError("admin", [])

        if "username" in updates:
            self._validate_username(updates["username"])
            user.username = updates["username"]

        if "email" in updates:
            self._validate_email(updates["email"])
            user.email = updates["email"]

        if "status" in updates and admin is not None:
            if not admin.has_permission("manage_users"):
                raise AuthorizationError("manage_users", admin.permissions)
            user.status = Status[updates["status"]]

        user.update()
        self.repository.save(user)
        self._cache.pop(user_id, None)
        return user

    def delete_user(self, user_id: int, admin: Admin) -> bool:
        """Delete user (admin only)."""
        if not admin.has_permission("delete"):
            raise AuthorizationError("delete", admin.permissions)

        user = self.get_user(user_id)
        user.status = Status.DELETED
        user.update()
        self.repository.save(user)
        self._cache.pop(user_id, None)
        return True

    def list_users(
        self,
        status: Optional[Status] = None,
        limit: int = 100,
        offset: int = 0,
    ) -> List[User]:
        """List users with optional filtering."""
        users = self.repository.list_all()

        if status is not None:
            users = [u for u in users if u.status == status]

        return users[offset : offset + limit]

    def search_users(self, query: str) -> List[User]:
        """Search users by username or email."""
        query_lower = query.lower()
        users = self.repository.list_all()
        return [
            u
            for u in users
            if query_lower in u.username.lower() or query_lower in u.email.lower()
        ]


class AsyncUserService:
    """Async version of user service for high-performance scenarios."""

    def __init__(self, config: Config) -> None:
        self.config = config
        self._users: Dict[int, User] = {}
        self._lock = asyncio.Lock()

    async def get_user(self, user_id: int) -> User:
        """Get user asynchronously."""
        async with self._lock:
            if user_id not in self._users:
                raise NotFoundError("User", user_id)
            return self._users[user_id]

    async def create_user(self, username: str, email: str) -> User:
        """Create user asynchronously."""
        user_id = User.generate_id()
        user = User(user_id, username, email)

        async with self._lock:
            self._users[user_id] = user

        return user

    async def batch_create_users(
        self,
        user_data: List[Dict[str, str]],
    ) -> List[User]:
        """Create multiple users concurrently."""
        tasks = [
            self.create_user(data["username"], data["email"]) for data in user_data
        ]
        return await asyncio.gather(*tasks)

    async def stream_users(self) -> AsyncIterator[User]:
        """Stream users one at a time."""
        for user in self._users.values():
            yield user
            await asyncio.sleep(0)

    async def process_user_batch(
        self,
        user_ids: List[int],
        processor: Any,
    ) -> List[ServiceResult]:
        """Process batch of users with given processor."""
        results: List[ServiceResult] = []

        for user_id in user_ids:
            try:
                user = await self.get_user(user_id)
                result = await processor(user)
                results.append(ServiceResult(success=True, data=result))
            except NotFoundError as error:
                results.append(ServiceResult(success=False, error=str(error)))
            except Exception as error:
                results.append(ServiceResult(success=False, error=f"Processing failed: {error}"))

        return results


def create_user_service(config: Config) -> UserService:
    """Factory function for creating UserService instances."""
    from mypackage.repositories import InMemoryUserRepository
    repository = InMemoryUserRepository()
    return UserService(repository, config)


async def run_user_migration(
    source: AsyncUserService,
    target: AsyncUserService,
) -> int:
    """Migrate users from source to target service."""
    count = 0
    async for user in source.stream_users():
        await target.create_user(user.username, user.email)
        count += 1
    return count
