#!/usr/bin/env python3
import crypt
h = crypt.crypt("password", crypt.mksalt(crypt.METHOD_MD5))
print(h)
