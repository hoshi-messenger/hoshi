# Hoshi

Monorepo of the Hoshi network/messenger. Still in early development so it's not usable as a messenger just yet.

## ToDo / Next Steps

- [ ] Notify and sync instead of sending messages directly, on startup notify all contacts
- [ ] Allow / Show messages from unknown people, change titleline so that instead of edit/delete we have add/block

## Backlog

- [ ] Audio calls, start with a Ring message, the other party may then send back an Accept/Reject message directly, once the call is open just spam the other side with regular G711 u-Law sample data and output it with rodio or something like that
- [ ] Use snow for actual crypto
- [ ] Work on the merkle-tree based sync with gossip support
- [ ] Add some clientlib unit tests
- [ ] Add some relay unit tests
- [ ] Add integration tests between clientlib <-> relay

## Polish
Those aren't that important since it's just a prototype, but if there's time might implement
a couple of them since they shouldn't take too long

- [ ] Show arrows in chat bubbles only if they're the last (look at Telegram)
- [ ] Add timestamps before messages
- [ ] Add timestamps only if there's a pause of more than 5 minutes between messages
- [ ] Add an Application Icon
- [ ] Add right/long click context menu to contacts, should show Edit/Delete options

## Long-term polish
Those are more complicated non-essential tasks, will probably take a while until we get to them, though AI might help here.

- [ ] Build custom Emoji chooser, make it look nice on bare X11

## Completed

- [X] Add arrows to chat bubbles