%define __spec_install_post %{nil}
%define __os_install_post %{_dbpath}/brp-compress
%define debug_package %{nil}

Name: ruuvi2iotcore
Summary: Ruuvi tag beacons to GCP iot core
Version: @@VERSION@@
Release: @@RELEASE@@%{?dist}
License: MIT
Group: Applications/Internet
Source0: %{name}-%{version}.tar.gz

BuildRoot: %{_tmppath}/%{name}-%{version}-%{release}-root

%description
%{summary}

%prep
%setup -q

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}
cp -a * %{buildroot}
# ls -laR %{buildroot}

%clean
rm -rf %{buildroot}

%files
%defattr(-,root,root,-)
%{_bindir}/ruuvi2iotcore
%license %{_docdir}/LICENSE
%doc %{_docdir}/CHANGELOG.md
%doc %{_docdir}/log4rs.yaml
%doc %{_docdir}/ruuvi2iotcore.yaml
%doc %{_docdir}/example_gateway_config.json

%post
if [ -x /usr/sbin/setcap ]; then
    setcap 'cap_net_raw,cap_net_admin+eip' %{_bindir}/ruuvi2iotcore
else
    chmod 1777 %{_bindir}/ruuvi2iotcore
fi
