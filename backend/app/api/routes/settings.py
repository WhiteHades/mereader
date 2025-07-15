"""
MeReader User Settings API Routes
"""
import logging
from datetime import datetime
from typing import Optional
from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session
from pydantic import BaseModel, Field
from app.db.sqlite import get_db
from app.db.models import Settings

router = APIRouter()
logger = logging.getLogger(__name__)

class SettingsUpdate(BaseModel):
    """Request model for updating settings"""
    theme: Optional[str] = Field(None, description="UI theme (light/dark/sepia)")
    font_family: Optional[str] = Field(None, description="Font family name")
    font_size: Optional[float] = Field(None, description="Font size in points", gt=0)
    line_spacing: Optional[float] = Field(None, description="Line spacing multiplier", gt=0)
    margin_size: Optional[float] = Field(None, description="Margin size in logical pixels", ge=0)
    text_alignment: Optional[str] = Field(None, description="Text alignment (left/justify/right)")

class SettingsResponse(BaseModel):
    """Response model for settings"""
    id: str
    theme: str
    font_family: str
    font_size: float
    line_spacing: float
    margin_size: float
    text_alignment: str
    updated_at: datetime

    class Config:
        """Pydantic config"""
        from_attributes = True

@router.get("/", response_model=SettingsResponse)
async def get_settings(db: Session = Depends(get_db)):
    """
    Get user settings
    """
    try:
        settings = db.query(Settings).first()

        if not settings:
            settings = Settings()
            db.add(settings)
            db.commit()
            db.refresh(settings)

        return settings

    except Exception as e:
        logger.error(f"Failed to get settings: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get settings: {str(e)}"
        )

@router.put("/", response_model=SettingsResponse)
async def update_settings(settings_update: SettingsUpdate, db: Session = Depends(get_db)):
    """
    Update user settings
    """
    try:
        settings = db.query(Settings).first()

        if not settings:
            settings = Settings()
            db.add(settings)

        update_data = settings_update.dict(exclude_unset=True)
        for field, value in update_data.items():
            if value is not None:
                setattr(settings, field, value)

        # updating timestamp
        settings.updated_at = datetime.utcnow()

        db.commit()
        db.refresh(settings)

        return settings

    except Exception as e:
        logger.error(f"Failed to update settings: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to update settings: {str(e)}"
        )

@router.post("/reset", response_model=SettingsResponse)
async def reset_settings(db: Session = Depends(get_db)):
    """
    Reset settings to default values
    """
    try:
        settings = db.query(Settings).first()

        if not settings:
            settings = Settings()
            db.add(settings)
        else:
            # defaults
            settings.theme = "dark"
            settings.font_family = "Default"
            settings.font_size = 16.0
            settings.line_spacing = 1.5
            settings.margin_size = 16.0
            settings.text_alignment = "left"

        # updating timestamp
        settings.updated_at = datetime.utcnow()

        db.commit()
        db.refresh(settings)

        return settings

    except Exception as e:
        logger.error(f"Failed to reset settings: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to reset settings: {str(e)}"
        )