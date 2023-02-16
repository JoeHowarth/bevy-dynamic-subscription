# bevy-dynamic-subscription

**This crate is working as a POC, but still needs additional polish -- PRs welcome!**

Allows clients to subscribe to dynamic queries using [bevy-ecs-dynamic](https://github.com/jakobhellermann/bevy_ecs_dynamic) which are run every frame. 
The query results are serialized to json and sent to the client over WebSocket. 

This allows for clients to determine which part of the data model should be synced at any given time. 
By supporting `Changed` or `Added` filters, less data needs to be transfered to stay in-sync.

Enabling the client to determine what data is needs facilitates a faster development loop since changes on the backend are no longer required.


