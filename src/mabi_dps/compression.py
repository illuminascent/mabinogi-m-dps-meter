"""Optional Brotli adapter; replay itself has no third-party requirements."""
import importlib

MAX_OUTPUT = 32 * 1024 * 1024


def brotli_decoder():
    """Return a decoder only if the installed binding offers bounded output.

    Never fall back to brotli.decompress(), which expands an entire frame before
    its size can be checked. This adapter accepts the newer process() API only.
    """
    try:
        module = importlib.import_module('brotli')
    except ImportError:
        return None
    try:
        module.Decompressor().process(b'', output_buffer_limit=1)
    except TypeError:
        return None

    def decode(data):
        decoder = module.Decompressor()
        try:
            output = bytearray(decoder.process(data, output_buffer_limit=MAX_OUTPUT + 1))
            while not decoder.is_finished() and len(output) <= MAX_OUTPUT:
                chunk = decoder.process(b'', output_buffer_limit=MAX_OUTPUT + 1 - len(output))
                if not chunk:
                    raise ValueError('Incomplete Brotli frame')
                output.extend(chunk)
        except module.error as exc:
            raise ValueError(f'Invalid Brotli frame: {exc}') from exc
        if len(output) > MAX_OUTPUT:
            raise ValueError('Brotli output exceeds 32 MiB')
        return bytes(output)

    return decode
