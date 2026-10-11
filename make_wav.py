import math, struct
f = open('/mnt/nexa_disk/data/test.wav', 'wb')
sample_rate = 8000
duration = 3.0
n_samples = int(sample_rate * duration)
f.write(b'RIFF' + struct.pack('<I', 36 + n_samples) + b'WAVEfmt ')
f.write(struct.pack('<IHHIIHH', 16, 1, 1, sample_rate, sample_rate, 1, 8))
f.write(b'data' + struct.pack('<I', n_samples))
for i in range(n_samples):
    val = int(127.0 * math.sin(2.0 * math.pi * 440.0 * (i / sample_rate))) + 128
    f.write(bytes([val]))
f.close()
