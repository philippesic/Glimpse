# zbus

This package is based on zbus 5.19.0.

The AccessKit AT-SPI adapter sends large text replies over Unix streams in bounded chunks under the existing connection write lock. The D-Bus string header and body encoding are unchanged. Ordinary typed messages and transports that require a complete message use the original reply path.
