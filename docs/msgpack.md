For serialization we make heavy use of msgpack via the rmp_serde
crate, currently JSON seems to be the default choice but for this
project it would have meant a couple of serious disadvantages. The
biggest one is missing support for binary data, for smaller payloads
you could get away with just putting Base64 encoded strings into the
JSON but that is quite inefficient both for storage as well as now
requiring you to decode twice. You might compress the JSON to work
around at least the space overhead somewhat, but now you've added
another big complexity and you now have the issue that
in most cases the base64 string contains an already compressed payload,
like an image, audio or video data so you only really compress the
overhead introduced by a suboptimal format. Using a format like msgpack
makes all of this much simpler, you can just serialize a Vec<u8> and
get a reasonably efficient representation.

Protobuf would have also worked fine here but the whole schema-first
approach would have made the integration much more complicated since
we now have to compile the `.proto` files and the main benefit
of replacing field names with field indices wouldn't improve things
much for Hoshi where we have few fields with big values.
