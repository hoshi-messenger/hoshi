# Hoshi

Monorepo of the Hoshi network/messenger. Still in early development so it's not usable as a messenger just yet.

## ToDo / Next Steps

- [ ] Use snow for actual crypto
- [ ] Work on the merkle-tree based sync with gossip support

## Backlog

- [ ] Add some clientlib unit tests
- [ ] Add some relay unit tests
- [ ] Add integration tests between clientlib <-> relay

## Polish
Those aren't that important since it's just a prototype, but if there's time might implement
a couple of them since they shouldn't take too long

- [ ] Allow / Show messages from unknown people, change titleline so that instead of edit/delete we have add/block
- [ ] Show arrows in chat bubbles only if they're the last (look at Telegram)
- [ ] Add timestamps before messages
- [ ] Add timestamps only if there's a pause of more than 5 minutes between messages
- [ ] Add an Application Icon
- [ ] Add right/long click context menu to contacts, should show Edit/Delete options
- [ ] Contact status indicators
- [ ] Show relay metrics on the landing page and send it via JSON
- [ ] Build public global relay dashboard
- [ ] Show last message instead of public key, in the chat view show status 

## Long-term polish
Those are more complicated non-essential tasks, will probably take a while until we get to them, though AI might help here.

- [ ] Build custom Emoji chooser, make it look nice on bare X11
- [ ] Typing indicator

## Completed

- [X] Add arrows to chat bubbles
- [X] Notify and sync instead of sending messages directly, on startup notify all contacts
- [X] Audio calls, start with a Ring message, the other party may then send back an Accept/Reject message directly, once the call is open just spam the other side with regular G711 u-Law sample data and output it with rodio or something like that
- [X] Abstract audio interface, shouldn't have Rodio in clientlib
- [X] Make the clientlib reconnection logic more robust
- [X] Simple bots (Echo / Jukebox)

## License

Unless otherwise stated all source codes in this repository is under the MPL 2.0 license which is included here.