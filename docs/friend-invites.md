# Friend favourites and personal invites

All routes require a user session. Favourite IDs are private to their owner.

* `GET /users/@me/favourites` returns the IDs of the caller's favourite friends.
* `PUT /users/@me/favourites/{id}` marks an existing friend; `DELETE` removes the mark. Removing or blocking the friendship clears it.
* `POST /users/@me/friend-invites` creates a 24-hour token and returns `token` and `expires_at`. Creating another token invalidates the previous one. Only a hash is stored.
* `DELETE /users/@me/friend-invites` revokes the caller's current token.
* `POST /friend-invites` with `{ "token": "..." }` sends a request from the authenticated caller to the token owner using the ordinary relationship rules. Expired or revoked tokens return unknown invite.

The desktop and mobile apps share an HTTPS link with the token in the fragment: `/friend-invite/#<token>`. The separate landing page opens `voics://friend-invite/<token>`; opening it requires the app and sign-in. Links received while signed out are handled after sign-in. Tokens are bearer credentials: share them only with the intended recipients.
