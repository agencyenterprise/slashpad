// Login-item shim: wraps SMAppService in native @try/@catch so an
// Obj-C exception (e.g. unrecognized selector on pre-13 runtimes
// where +mainApp genuinely isn't there) returns a status code instead
// of aborting the Rust process.
//
// We also use objc_getClass + class_getClassMethod to probe for
// +[SMAppService mainApp] before dispatch — avoiding the raise
// entirely in the common "API not present" case.
//
// Return values:
//   0  success
//   1  SMAppService class not available (pre-macOS 13)
//   2  +mainApp selector not recognised
//   3  mainApp returned nil
//   4  register/unregister returned NO
//   5  Obj-C exception caught

#import <Foundation/Foundation.h>
#import <objc/runtime.h>
#import <objc/message.h>

typedef id (*msg_send_t)(id, SEL);
typedef BOOL (*msg_send_err_t)(id, SEL, NSError **);
typedef long (*msg_send_status_t)(id, SEL);

static int slashpad_sm_call(SEL action) {
    @try {
        Class cls = objc_getClass("SMAppService");
        if (!cls) return 1;

        SEL mainAppSel = sel_registerName("mainApp");
        Method m = class_getClassMethod(cls, mainAppSel);
        if (!m) return 2;

        msg_send_t send = (msg_send_t)objc_msgSend;
        id service = send((id)cls, mainAppSel);
        if (!service) return 3;

        msg_send_err_t sendErr = (msg_send_err_t)objc_msgSend;
        NSError *error = nil;
        BOOL ok = sendErr(service, action, &error);
        if (!ok) {
            if (error) {
                NSLog(@"[slashpad] SMAppService call failed: %@", error);
            }
            return 4;
        }
        return 0;
    } @catch (NSException *exc) {
        NSLog(@"[slashpad] SMAppService raised: %@: %@", exc.name, exc.reason);
        return 5;
    }
}

int slashpad_login_item_register(void) {
    return slashpad_sm_call(sel_registerName("registerAndReturnError:"));
}

int slashpad_login_item_unregister(void) {
    return slashpad_sm_call(sel_registerName("unregisterAndReturnError:"));
}

// Returns 1 if +[SMAppService mainApp] is dispatchable without raise,
// 0 otherwise. Used to decide whether to surface the Launch-at-login
// checkbox at all.
int slashpad_login_item_supported(void) {
    @try {
        Class cls = objc_getClass("SMAppService");
        if (!cls) return 0;
        SEL mainAppSel = sel_registerName("mainApp");
        Method m = class_getClassMethod(cls, mainAppSel);
        return m ? 1 : 0;
    } @catch (NSException *exc) {
        return 0;
    }
}
