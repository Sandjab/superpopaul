"""Génère app-icon.png (1024×1024, aplat orange popcorn) sans dépendance."""
import struct, zlib

W = H = 1024
row = b"\x00" + bytes([245, 166, 35, 255]) * W  # filtre None + RGBA #f5a623

def chunk(tag: bytes, data: bytes) -> bytes:
    return (struct.pack(">I", len(data)) + tag + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(row * H))
       + chunk(b"IEND", b""))
with open("app-icon.png", "wb") as f:
    f.write(png)
print("app-icon.png écrit")
