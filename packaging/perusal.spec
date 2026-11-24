Name:           perusal
Version:        0.1.0
Release:        1%{?dist}
Summary:        Fast merge-request and pull-request reviewer
License:        MIT OR Apache-2.0
Requires:       git
%description
Filter, batch and graph based review for GitLab and GitHub.
%install
install -Dm755 %{_sourcedir}/perusal %{buildroot}%{_bindir}/perusal
install -Dm755 %{_sourcedir}/perusal-cli %{buildroot}%{_bindir}/perusal-cli
install -Dm644 %{_sourcedir}/perusal.desktop %{buildroot}%{_datadir}/applications/perusal.desktop
%files
%{_bindir}/perusal
%{_bindir}/perusal-cli
%{_datadir}/applications/perusal.desktop
