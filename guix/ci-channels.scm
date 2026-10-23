;; The binary builder uses the same Guix revision as ../System/channels.scm.
;; The resulting binary closure is exported, so consumers do not need to
;; reproduce the build environment or compile Minde when their channels change.
(list (channel
       (name 'guix)
       (url "https://codeberg.org/guix/guix.git")
       (commit "0802546301e0a9fab4d43b872ddac96c753a2430")
       (introduction
        (make-channel-introduction
         "9edb3f66fd807b096b48283debdcddccfea34bad"
         (openpgp-fingerprint
          "BBB0 2DDF 2CEA F6A8 0D1D E643 A2A0 6DF2 A33A 54FA")))))
