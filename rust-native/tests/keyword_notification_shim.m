// Explicit SDK fixture: typed retained application objects with immutable
// instance tokens. Never creates, discovers or activates a foreign process.
// MRC on purpose: this test subclass bypasses NSRunningApplication's private
// constructor/destructor and owns only two scalar ivars.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdint.h>
#include <unistd.h>
extern bool ruycast_keyword_fixture_match_notification_identity(void *,uint64_t,int32_t);
@interface RuycastKeywordFixtureApplication : NSRunningApplication {
@public uint64_t token; int32_t fixturePID;
}
@end
@implementation RuycastKeywordFixtureApplication
- (pid_t)processIdentifier {return fixturePID;}
- (BOOL)isTerminated {return NO;}
- (BOOL)isEqual:(id)other {
    if(other==self)return YES;
    if([other isKindOfClass:[RuycastKeywordFixtureApplication class]]) {
        RuycastKeywordFixtureApplication *app=other;
        return app->token==token&&app->fixturePID==fixturePID;
    }
    return token==UINT64_MAX&&[[NSRunningApplication currentApplication] isEqual:other];
}
- (NSUInteger)hash {return (NSUInteger)token;}
- (void)dealloc {object_dispose(self);}
@end
__attribute__((visibility("default"))) void *ruycast_keyword_fixture_notification_application(uint64_t token,int32_t pid){
    RuycastKeywordFixtureApplication *app=class_createInstance([RuycastKeywordFixtureApplication class],0);
    app->token=token;app->fixturePID=pid;return app;
}
__attribute__((visibility("default"))) bool ruycast_keyword_fixture_app_matches_notification(void *target,void *value){
    id app=(id)value;
    if([app isKindOfClass:[RuycastKeywordFixtureApplication class]]){
        RuycastKeywordFixtureApplication *fixture=app;
        return ruycast_keyword_fixture_match_notification_identity(target,fixture->token,fixture->fixturePID);
    }
    if([[NSRunningApplication currentApplication] isEqual:app])
        return ruycast_keyword_fixture_match_notification_identity(target,UINT64_MAX,getpid());
    return false;
}
