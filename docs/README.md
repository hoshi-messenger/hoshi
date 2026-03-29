Hey there, this file describes some high-level architectural ideas about Hoshi
and should point into the direction I'd like Hoshi to go into.

# Authentication

The idea here is that we have a centralized control plane that both clients and
relays can register with. this will allow us to for example verify an E-Mail
during registration for example as a way to reduce spam. To authenticate a relay
the control plane signs the certificate of the relay, same goes for clients which
use client certificates whenever connecting to a relay, that way both parties can
check that each other has been vetted before sending any data.

This allows us to reuse existing TLS libraries for transport layer
security, while also allowing multiple control planes in the future by issuing
intermediate certificates to trusted operators.

# Relays

Relays should be as stateless as possible, their primary function is forwarding
opaque packets from client to client, that's it. They should never know anything
about groups, calls, images or anything like that. They listen for incoming
WebSocket connections, make sure that the client was verified by the control
plane and then forward those packages, right now each relay is their own network,
this is due to me focusing on client work since multiple relays only really
become necessary once there are enough users that a single machine can't handle it,
which is still quite far in the future. Once this happens we'll have to figure
out how to handle routing, currently my idea is to have each relay also open
a client connection to itself and join a group chat with all other relays, there
we just broadcast which public key is connected to which relay, that way each
relay can build a complete routing table and for bootstrapping we can use a list
from the control plane so we know enough other relays for the gossip protocol
to work.

This idea of using a special group chat for system information is also something
we should try for the relay list itself, that way each client just joins this
group chat in the background and thereby automatically has an up-to-date list of
all the relays currently online, this should also contain some rough health
metrics for each relay so that clients can more intelligently figure out which
relay to connect to.

It is important that relays get as little information as possible since one core
idea is that anyone should be able to setup a relay, so we need to ensure that
this doesn't negatively impact user privacy. For this we should be able to add
a simple 2-hop onion routing layer to hide metadata from the relays.

# Clients

Now regarding clients, the transport layer was already explained in the prior
section about relays, generally speaking, each client is identified by its
public key and can send messages to other clients. Right now it just sends those
messages to the relay it's connecting to but in the future I'd like to experiment
with automatically establishing a direct connection between 2 clients if there's
sustained high-activity but this should be completely hidden within the clientlib.

To simplify building various communication/signaling channels most user-level
features should be built on top of an immutable hierachical message store, let's
take a one on one chat for example, instead of just sending a text message to the
other party directly we instead insert a new message/event into
`/chat/{alice.public_key ^ bob.public_key}/`, after this we then trigger
a sync approach where 2 clients compare hashes and send missing messages to each
other. While this complicates the simple text message use case it allows us to
just build one set of syncronization operation and then build all sorts of other
channels on top of, by also storing the original signed message we can also use
a gossip based approach to disseminate group chats between all members.

# Groups

The current idea for groups/spaces is that the creator of a group creates a new
UUID which is used as the group identifier, something like `/group/{uuid}/` it
then inserts UserRole messages to indicate membership and permission level. once
another client decides to join it will then sync this path with the person that
invited them and gets a full member list to sync with in the future. Every member
can verify whether a given message was allowed or not and is required to drop any
messages from users without the right permissions.

# Public Groups

This approach about groups being a sort of shared secret between members creates
a problem, how do you allow public groups people can just join without being
specifically invited? To adress this the idea is to use GroupList bot, when
connecting to the control plane we also get a list of standardized bots and their
public key on the network which can handle various services. So if you made a
group and would like it to be public you just need to invite the GroupList bot
as a mod/admin, by giving it invite permission it can then automatically add
interested clients. An additional benefit would be that it'd be an always online
party with which to sync group messages with. The low-level interface should just
be special text messages we send to this bot, so we might send it a `/list` message
to which it replies with a list of all known groups, or a `/search keyword` message
and finally a `/join UUID` message. To make this easy to use the client should do
this in the background and show the user a nice interface. To provide a nice listing
the bot should also check for various metadata messages that set the groups title/description
and it also already knows how many members a given group has for a useful directory.

# User Profiles

Since the network itself only knows public keys we need a way to figure out how
a user actually wants to be called, for this we reuse the existing sync approach
where we have a path like `/user/{public_key}/` where each client will insert
messages for setting the user name and other profile data, when we add a new contact
we then try and sync this profile path to show a username instead of the public key
or the emoji placeholder we generate based on the public key. Once we have file/image
support we should also be able to easily extend this to allow for avatar pictures
by just posting an image into this channel.
