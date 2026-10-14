/*
 * The controls the machine shows over its own screen on an iPhone or an iPad.
 *
 * See KMPhoneShell.m.
 */

#import <Foundation/Foundation.h>

/* Names the directory that holds settings.json. Called from main, before SDL starts. */
void km_phone_shell_configure(NSString *support);

/*
 * Puts the controls over the machine's window. Called from Rust, through `ioscfg::attach_shell`,
 * once the renderer exists. Takes SDL's `UIWindow`. A second call does nothing.
 */
void km_phone_shell_attach(void *window);
