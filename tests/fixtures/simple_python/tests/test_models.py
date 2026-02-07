"""Tests for data models."""

import pytest
from mypackage.models import User, Admin, Status, Config, Address, Entity


class TestUser:
    """Test suite for User model."""

    def test_create_user(self) -> None:
        """Test user creation."""
        user = User(1, "testuser", "test@example.com")
        assert user.username == "testuser"
        assert user.email == "test@example.com"
        assert user.status == Status.PENDING

    def test_user_is_active(self) -> None:
        """Test is_active property."""
        user = User(1, "testuser", "test@example.com")
        assert not user.is_active
        user.activate()
        assert user.is_active

    def test_user_password(self) -> None:
        """Test password hashing and verification."""
        user = User(1, "testuser", "test@example.com")
        user.set_password("secret123")
        assert user.verify_password("secret123")
        assert not user.verify_password("wrongpassword")

    def test_user_to_dict(self) -> None:
        """Test serialization to dict."""
        user = User(1, "testuser", "test@example.com")
        result = user.to_dict()
        assert result["id"] == 1
        assert result["username"] == "testuser"
        assert result["status"] == "PENDING"

    @pytest.mark.parametrize(
        "status",
        [Status.PENDING, Status.ACTIVE, Status.SUSPENDED],
    )
    def test_user_status_transitions(self, status: Status) -> None:
        """Test various status values."""
        user = User(1, "testuser", "test@example.com", status)
        assert user.status == status


class TestAdmin:
    """Test suite for Admin model."""

    def test_admin_has_default_permissions(self) -> None:
        """Test admin has default permissions."""
        admin = Admin(1, "admin", "admin@example.com")
        assert admin.has_permission("read")
        assert admin.has_permission("write")
        assert admin.has_permission("delete")

    def test_admin_grant_permission(self) -> None:
        """Test granting permissions."""
        admin = Admin(1, "admin", "admin@example.com", permissions=["read"])
        admin.grant_permission("write")
        assert admin.has_permission("write")

    def test_admin_revoke_permission(self) -> None:
        """Test revoking permissions."""
        admin = Admin(1, "admin", "admin@example.com")
        admin.revoke_permission("delete")
        assert not admin.has_permission("delete")


class TestConfig:
    """Test suite for Config dataclass."""

    def test_config_defaults(self) -> None:
        """Test config default values."""
        config = Config(database_url="postgres://localhost", api_key="secret")
        assert config.debug is False
        assert config.max_connections == 100
        assert config.allowed_hosts == []

    def test_config_validate_success(self) -> None:
        """Test successful validation."""
        config = Config(database_url="postgres://localhost", api_key="secret")
        assert config.validate() is True

    def test_config_validate_missing_url(self) -> None:
        """Test validation fails with missing URL."""
        config = Config(database_url="", api_key="secret")
        with pytest.raises(ValueError, match="database_url"):
            config.validate()


@pytest.fixture
def sample_user() -> User:
    """Fixture providing a sample user."""
    return User(1, "fixture_user", "fixture@example.com")


@pytest.fixture
def sample_admin() -> Admin:
    """Fixture providing a sample admin."""
    return Admin(1, "fixture_admin", "admin@example.com")


def test_entity_registry() -> None:
    """Test entity registry tracking."""
    entity = Entity(999)
    assert Entity.get_by_id(999) is entity


def test_address_immutability() -> None:
    """Test frozen dataclass."""
    address = Address("123 Main St", "City", "Country")
    with pytest.raises(Exception):
        address.street = "New Street"  # type: ignore[misc]
