# MeReader

Local LLM powered E-book reader for EPUB files.

## Setup

**Requirements:** Python 3.8+, Node.js 16+

### Backend

```bash
cd backend
python -m venv venv
# windows: venv\Scripts\activate
# linux/mac: source venv/bin/activate
source venv/bin/activate
pip install -r requirements.txt
python -m app.api.main
```

### Frontend

```bash
cd frontend
npm install
npm run dev
```

**Desktop app:** `npm run tauri build`

## Usage

1. Start backend and frontend
2. Upload EPUB files
3. Read books and ask any chosen LLM about the things read.

## Structure

```
backend/     # Python FastAPI
frontend/    # Vue.js + Tauri
```

## Common Issues

- **Backend won't start:** Check virtual environment is activated
- **Frontend errors:** Delete `node_modules`, run `npm install`
- **Upload fails:** Only EPUB files supported

## Testing

```bash
# Backend
cd backend && pytest

# Frontend
cd frontend && npm test
```

## License

This project is open source. Feel free to use and modify as needed.
