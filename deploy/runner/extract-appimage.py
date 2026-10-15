"""Extract validated SquashFS without executing the architecture-specific AppImage runtime."""
import mmap
import struct
import subprocess
import sys

with open(sys.argv[1], 'rb') as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as data:
    offset = -1
    while True:
        offset = data.find(b'hsqs', offset + 1)
        if offset < 0:
            raise SystemExit('No valid SquashFS v4 superblock found')
        if offset + 96 > len(data):
            continue
        major, minor = struct.unpack_from('<HH', data, offset + 28)
        used = struct.unpack_from('<Q', data, offset + 40)[0]
        block_size = struct.unpack_from('<I', data, offset + 12)[0]
        if (major, minor) == (4, 0) and 96 <= used <= len(data) - offset and block_size in (4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288, 1048576):
            subprocess.run(['unsquashfs', '-processors', '2', '-o', str(offset), '-d', sys.argv[2], sys.argv[1]], check=True)
            break
