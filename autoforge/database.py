from sqlalchemy.ext.asyncio import AsyncSession, create_async_engine, async_sessionmaker
from sqlalchemy.orm import DeclarativeBase
from autoforge.config import settings

# Convert sync postgres URL to async
_db_url = settings.database_url.replace("postgresql://", "postgresql+asyncpg://")
_db_url = _db_url.replace("postgres://", "postgresql+asyncpg://")

engine = create_async_engine(_db_url, echo=settings.debug, pool_pre_ping=True)
AsyncSessionLocal = async_sessionmaker(engine, class_=AsyncSession, expire_on_commit=False)


class Base(DeclarativeBase):
    pass


async def get_db() -> AsyncSession:
    async with AsyncSessionLocal() as session:
        try:
            yield session
        finally:
            await session.close()
