"""Tests for service layer."""

import pytest
from mypackage.services import UserService, AsyncUserService, ServiceResult
from mypackage.models import User, Admin, Config, Status
from mypackage.repositories import InMemoryUserRepository
from mypackage.errors import ValidationError, NotFoundError, AuthorizationError


class TestUserService:
    """Test suite for UserService."""

    @pytest.fixture
    def service(self) -> UserService:
        """Create service with test dependencies."""
        repo = InMemoryUserRepository()
        config = Config(database_url="test://", api_key="test")
        return UserService(repo, config)

    def test_create_user(self, service: UserService) -> None:
        """Test user creation."""
        user = service.create_user("testuser", "test@example.com", "Password123")
        assert user.username == "testuser"
        assert user.email == "test@example.com"

    def test_create_user_invalid_username(self, service: UserService) -> None:
        """Test validation for short username."""
        with pytest.raises(ValidationError) as exc_info:
            service.create_user("ab", "test@example.com", "Password123")
        assert exc_info.value.field == "username"

    def test_create_user_invalid_email(self, service: UserService) -> None:
        """Test validation for invalid email."""
        with pytest.raises(ValidationError) as exc_info:
            service.create_user("testuser", "invalid-email", "Password123")
        assert exc_info.value.field == "email"

    def test_create_user_weak_password(self, service: UserService) -> None:
        """Test validation for weak password."""
        with pytest.raises(ValidationError) as exc_info:
            service.create_user("testuser", "test@example.com", "weak")
        assert exc_info.value.field == "password"

    def test_get_user_not_found(self, service: UserService) -> None:
        """Test getting non-existent user."""
        with pytest.raises(NotFoundError) as exc_info:
            service.get_user(999)
        assert exc_info.value.resource_type == "User"

    def test_authenticate_success(self, service: UserService) -> None:
        """Test successful authentication."""
        user = service.create_user("testuser", "test@example.com", "Password123")
        user.activate()
        authenticated = service.authenticate("testuser", "Password123")
        assert authenticated.entity_id == user.entity_id

    def test_delete_user_requires_admin(self, service: UserService) -> None:
        """Test delete requires admin permission."""
        user = service.create_user("testuser", "test@example.com", "Password123")
        admin = Admin(2, "admin", "admin@example.com", permissions=["read"])
        with pytest.raises(AuthorizationError):
            service.delete_user(user.entity_id, admin)


class TestAsyncUserService:
    """Test suite for AsyncUserService."""

    @pytest.fixture
    def async_service(self) -> AsyncUserService:
        """Create async service."""
        config = Config(database_url="test://", api_key="test")
        return AsyncUserService(config)

    @pytest.mark.asyncio
    async def test_create_user_async(self, async_service: AsyncUserService) -> None:
        """Test async user creation."""
        user = await async_service.create_user("asyncuser", "async@example.com")
        assert user.username == "asyncuser"

    @pytest.mark.asyncio
    async def test_get_user_async(self, async_service: AsyncUserService) -> None:
        """Test async user retrieval."""
        created = await async_service.create_user("asyncuser", "async@example.com")
        retrieved = await async_service.get_user(created.entity_id)
        assert retrieved.username == created.username

    @pytest.mark.asyncio
    async def test_batch_create_users(self, async_service: AsyncUserService) -> None:
        """Test batch user creation."""
        user_data = [
            {"username": "user1", "email": "user1@example.com"},
            {"username": "user2", "email": "user2@example.com"},
            {"username": "user3", "email": "user3@example.com"},
        ]
        users = await async_service.batch_create_users(user_data)
        assert len(users) == 3

    @pytest.mark.asyncio
    async def test_stream_users(self, async_service: AsyncUserService) -> None:
        """Test async user streaming."""
        await async_service.create_user("user1", "user1@example.com")
        await async_service.create_user("user2", "user2@example.com")

        users = []
        async for user in async_service.stream_users():
            users.append(user)

        assert len(users) == 2


class TestServiceResult:
    """Test ServiceResult wrapper."""

    def test_success_result(self) -> None:
        """Test successful result."""
        result = ServiceResult(success=True, data={"key": "value"})
        assert result.success is True
        assert result.data == {"key": "value"}
        assert result.error is None

    def test_failure_result(self) -> None:
        """Test failure result."""
        result = ServiceResult(success=False, error="Something went wrong")
        assert result.success is False
        assert result.error == "Something went wrong"
